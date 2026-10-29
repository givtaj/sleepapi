#!/usr/bin/env python3
"""Check a locally built Linux image using Docker and Python's standard library."""

import http.client
import json
import socket
import subprocess
import sys
import time
import uuid


ORIGIN = "http://localhost:5173"
SHUTDOWN_GRACE_MS = 500


def docker(*args, timeout=20, check=True):
    return subprocess.run(
        ["docker", *args], capture_output=True, text=True, timeout=timeout, check=check
    ).stdout.strip()


def require(condition, message):
    if not condition:
        raise AssertionError(message)


def inspect(container):
    return json.loads(docker("inspect", container))[0]


def request(port, method, path, payload=None, headers=None):
    headers = dict(headers or {})
    if payload is not None:
        headers["Content-Type"] = "application/json"
    connection = http.client.HTTPConnection("127.0.0.1", port, timeout=2)
    try:
        connection.request(method, path, json.dumps(payload) if payload is not None else None, headers)
        response = connection.getresponse()
        body = response.read()
        return response.status, dict((k.lower(), v) for k, v in response.getheaders()), body
    finally:
        connection.close()


def start(image, containers):
    name = "sleepapi-smoke-" + uuid.uuid4().hex
    containers[name] = None  # Also recover our container if `docker run` times out.
    container = docker(
        "run", "--detach", "--pull=never", "--name", name,
        "--publish", "127.0.0.1::3000",
        "--env", "SLEEPAPI_CORS_ORIGINS=" + ORIGIN,
        "--env", f"SLEEPAPI_SHUTDOWN_GRACE_MS={SHUTDOWN_GRACE_MS}", image,
    )
    containers[name] = container
    details = inspect(container)
    user = details["Config"]["User"].split(":")[0]
    require(user not in ("", "0", "root"), "container must configure a non-root user")
    binding = details["NetworkSettings"]["Ports"]["3000/tcp"][0]
    require(binding["HostIp"] == "127.0.0.1", "smoke container must bind only loopback")
    port = int(binding["HostPort"])
    deadline = time.monotonic() + 15
    while time.monotonic() < deadline:
        try:
            status, _, body = request(port, "GET", "/health")
            if status == 200 and json.loads(body) == {"status": "ok"}:
                return container, port
        except (OSError, http.client.HTTPException, ValueError):
            pass
        time.sleep(0.1)
    raise AssertionError("container did not become healthy within 15 seconds")


def check_http(port):
    status, _, body = request(port, "GET", "/scenarios")
    require(status == 200 and len(json.loads(body)["scenarios"]) == 7, "scenario discovery failed")
    for route, method in [
        ("tokio", "tokio_sleep"), ("tokio-spawn", "tokio_spawn"),
        ("thread", "thread_sleep"), ("park", "thread_park"),
    ]:
        started = time.monotonic()
        status, headers, body = request(
            port, "POST", "/sleep/" + route,
            {"duration_ms": 20, "scenario": "delayed_failure"}, {"Origin": ORIGIN},
        )
        result = json.loads(body)
        require(status == 503 and headers.get("retry-after") == "1", route + ": failure status/header")
        require(result["simulated"] and result["method"] == method, route + ": wrong waiting method")
        require(result["error"]["code"] == "service_unavailable" and result["error"]["retryable"], route + ": error body")
        require(result["elapsed_ms"] >= 20 and time.monotonic() - started >= 0.02, route + ": returned too early")
        require(headers.get("access-control-allow-origin") == ORIGIN, route + ": missing CORS response")
        exposed = {token.strip().lower() for token in headers.get("access-control-expose-headers", "").split(",")}
        require({"retry-after", "www-authenticate", "allow"} <= exposed, route + ": missing exposed headers")

    status, headers, _ = request(port, "OPTIONS", "/sleep/tokio", headers={
        "Origin": ORIGIN, "Access-Control-Request-Method": "POST",
        "Access-Control-Request-Headers": "content-type, authorization",
    })
    require(200 <= status < 300 and headers.get("access-control-allow-origin") == ORIGIN, "CORS preflight failed")
    allowed = {token.strip().lower() for token in headers.get("access-control-allow-headers", "").split(",")}
    require({"content-type", "authorization"} <= allowed, "CORS request headers were not allowed")
    methods = {token.strip() for token in headers.get("access-control-allow-methods", "").split(",")}
    require("POST" in methods, "CORS did not allow POST")

    for expected, header, value in [(401, "www-authenticate", 'Bearer realm="sleepapi"'), (405, "allow", "GET")]:
        status, headers, _ = request(port, "POST", "/sleep/tokio", {"duration_ms": 0, "status_code": expected})
        require(status == expected and headers.get(header) == value, str(expected) + ": missing protocol header")
    status, headers, _ = request(port, "GET", "/sleep/tokio")
    require(status == 405 and headers.get("allow") == "POST", "real 405 must advertise POST")
    print("PASS: health, seven scenarios, four timed failure routes, CORS, and 401/405 headers")


def stop(container, expected):
    docker("stop", "--time", "3", container, timeout=10)
    state = inspect(container)["State"]
    require(not state["Running"] and state["ExitCode"] == expected,
            f"expected exit {expected}, got {state['ExitCode']} (137 means Docker killed it)")


def check_recovery_run(port):
    status, _, body = request(port, "POST", "/runs", {
        "sequence": [{"status_code": code, "duration_ms": 0} for code in (503, 503, 200)],
    })
    require(status == 201, "could not create a recovery run")
    run = json.loads(body)
    path = "/runs/" + run["run_id"]
    generation = run["generation"]
    for attempt, expected in enumerate((503, 503, 200), start=1):
        status, _, body = request(port, "POST", path + "/attempts", {"generation": generation})
        result = json.loads(body)
        require(status == expected and result["attempt"] == attempt, "recovery sequence mismatch")
        require(result["run_id"] == run["run_id"] and result["generation"] == generation,
                "recovery response has wrong run/generation")
    status, _, _ = request(port, "POST", path + "/attempts", {"generation": generation})
    require(status == 409, "exhausted sequence must reject further attempts")
    status, _, body = request(port, "GET", path + "/receipts")
    receipts = json.loads(body)["receipts"]
    require(status == 200 and len(receipts) == 3 and all(r["outcome"] == "completed" for r in receipts),
            "recovery receipts missing completed attempts")
    status, _, body = request(port, "POST", path + "/reset", {"generation": generation})
    reset = json.loads(body)
    require(status == 200 and reset["generation"] == generation + 1 and reset["retained_receipts"] == 0,
            "recovery reset did not advance generation and clear receipts")
    status, _, _ = request(port, "POST", path + "/attempts", {"generation": generation})
    require(status == 409, "stale generation must not consume a reset sequence")
    status, _, body = request(port, "POST", path + "/attempts", {"generation": reset["generation"]})
    require(status == 503 and json.loads(body)["attempt"] == 1, "reset sequence did not restart")
    status, _, body = request(port, "DELETE", path)
    require(status == 204 and not body, "recovery run deletion failed")
    status, _, _ = request(port, "GET", path)
    require(status == 404, "deleted run remains accessible")
    print("PASS: recovery sequence, receipts, exhaustion, guarded reset, and deletion")


def check_incomplete_body(container, port):
    with socket.create_connection(("127.0.0.1", port), timeout=2) as connection:
        connection.sendall(
            b"POST /sleep/tokio HTTP/1.1\r\nHost: localhost\r\n"
            b"Content-Type: application/json\r\nContent-Length: 100\r\n"
            b"Expect: 100-continue\r\n\r\n"
        )
        reply = b""
        deadline = time.monotonic() + 3
        while b"\r\n\r\n" not in reply and time.monotonic() < deadline:
            part = connection.recv(4096)
            require(bool(part), "connection closed before the request body was accepted")
            reply += part
            require(len(reply) <= 8192, "unexpectedly large interim response")
        require(reply.split(b"\r\n", 1)[0] == b"HTTP/1.1 100 Continue", "expected an active request via 100 Continue")
        connection.sendall(b'{"duration_ms":')
        started = time.monotonic()
        stop(container, 2)  # Keep the partial request open until the container exits.
        elapsed = time.monotonic() - started
    # Only one SIGTERM is sent, and exit 2 rules out Docker's later SIGKILL.
    # Forced exits must not depend on delivering a final log record.
    require(elapsed >= SHUTDOWN_GRACE_MS / 1000, "process exited before its grace period")
    print("PASS: incomplete request exits 2 after its grace period, without Docker SIGKILL")


def check_fixtures(port):
    fixture = '{ "message": "ready — café", "ok": true }\n'
    status, _, body = request(port, "POST", "/runs", {
        "sequence": [{"status_code": 503, "duration_ms": 20, "fixture": {
            "body": fixture, "content_type": "application/json",
            "headers": {"retry-after": "3", "x-request-id": "fixture-check"},
        }}],
    })
    require(status == 201, "fixture run creation failed")
    run = json.loads(body)
    path = "/runs/" + run["run_id"]
    started = time.monotonic()
    status, headers, body = request(port, "POST", path + "/attempts", {"generation": run["generation"]})
    require(status == 503 and body == fixture.encode("utf-8"), "fixture bytes/status changed")
    require(headers.get("content-type") == "application/json" and headers.get("retry-after") == "3"
            and headers.get("x-request-id") == "fixture-check", "fixture headers changed")
    require(time.monotonic() - started >= 0.02, "fixture returned before selected wait")
    status, _, body = request(port, "GET", path + "/receipts")
    receipt = json.loads(body)["receipts"][0]
    require(status == 200 and receipt["outcome"] == "completed" and receipt["attempt"] == 1,
            "fixture diagnostics missing from separate receipts")
    status, _, _ = request(port, "DELETE", path)
    require(status == 204, "fixture cleanup failed")
    print("PASS: exact UTF-8 fixture bytes, headers, delay, separate receipts, and cleanup")


def check_transport_faults(port):
    status, _, body = request(port, "POST", "/runs", {"sequence": [
        {"status_code": 503, "duration_ms": 0, "fault": {"type": "connection_drop"}},
        {"status_code": 200, "duration_ms": 0,
         "fixture": {"body": "aéxyz", "content_type": "text/plain"},
         "fault": {"type": "truncated_response", "after_bytes": 2}},
        {"status_code": 200, "duration_ms": 0, "fixture": {"body": "recovered"}},
    ]})
    require(status == 201, "fault run creation failed")
    run = json.loads(body)
    path = "/runs/" + run["run_id"]
    for fault in ("connection_drop", "truncated_response"):
        connection = http.client.HTTPConnection("127.0.0.1", port, timeout=2)
        try:
            connection.request("POST", path + "/attempts", json.dumps({"generation": run["generation"]}),
                               {"Content-Type": "application/json"})
            if fault == "connection_drop":
                try:
                    connection.getresponse()
                except http.client.RemoteDisconnected:
                    pass
                else:
                    raise AssertionError("connection-drop fault returned HTTP headers")
            else:
                response = connection.getresponse()
                require(response.status == 200 and response.getheader("Content-Length") == "6",
                        "truncation did not declare full UTF-8 length")
                try:
                    response.read()
                except http.client.IncompleteRead as error:
                    require(error.partial == "aéxyz".encode("utf-8")[:2], "truncated prefix differs")
                else:
                    raise AssertionError("truncation returned a complete body")
        finally:
            connection.close()
    status, _, body = request(port, "POST", path + "/attempts", {"generation": run["generation"]})
    require(status == 200 and body == b"recovered", "normal recovery after faults failed")
    status, _, body = request(port, "GET", path + "/receipts")
    receipts = json.loads(body)["receipts"]
    require(status == 200 and [r["outcome"] for r in receipts] ==
            ["fault_injected", "fault_injected", "completed"], "fault outcomes missing")
    require([r["injection_stage"] for r in receipts[:2]] == ["before_headers", "after_body_prefix"]
            and [r["injected_body_bytes"] for r in receipts[:2]] == [0, 2], "fault injection facts missing")
    status, _, _ = request(port, "DELETE", path)
    require(status == 204, "fault cleanup failed")
    print("PASS: connection drop, byte-exact truncated response, recovery, receipts, and cleanup")


def main():
    if len(sys.argv) != 2:
        raise SystemExit("Usage: python3 scripts/docker-smoke.py IMAGE")
    containers = {}
    try:
        container, port = start(sys.argv[1], containers)
        check_http(port)
        check_recovery_run(port)
        check_fixtures(port)
        check_transport_faults(port)
        stop(container, 0)
        print("PASS: normal Docker stop exits 0")
        container, port = start(sys.argv[1], containers)
        check_incomplete_body(container, port)
    except BaseException:
        for name, container in containers.items():
            try:
                print(docker("logs", "--tail", "30", container or name, check=False), file=sys.stderr)
            except (OSError, subprocess.SubprocessError):
                pass
        raise
    finally:
        for name, container in containers.items():
            try:
                container = container or docker("inspect", "--format", "{{.Id}}", name, check=False)
                if container:
                    docker("rm", "--force", container, check=False)
            except (OSError, subprocess.SubprocessError) as error:
                print(f"Could not clean up own smoke container {name}: {error}", file=sys.stderr)


if __name__ == "__main__":
    main()

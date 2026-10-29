#!/usr/bin/env python3
"""Exercise alpha.4 recovery with a bounded HTTP/1.1 retry client.

Uses only Python 3.9+ standard-library modules. Run against a tagged local
installation, or pass --binary to own a temporary loopback server. The API does
not expose a version: target_release identifies this example's contract, not a
verified server version. This is synthetic client evidence, not client sign-off.
"""

import argparse
from collections import deque
from datetime import datetime, timezone
import hashlib
import http.client
import ipaddress
import json
import os
from pathlib import Path
import queue
import re
import subprocess
import sys
import threading
import time
from urllib.parse import urlsplit


TIMEOUT_SECONDS = 2
MAX_BODY_BYTES = 65536
MAX_ATTEMPTS = 3
FAULT_BACKOFF_SECONDS = 0.05
TARGET_RELEASE = "v0.1.1-alpha.4"


def require(condition, message):
    if not condition:
        raise AssertionError(message)


def local_address(base_url):
    url = urlsplit(base_url)
    require(url.scheme == "http" and url.hostname is not None,
            "base URL must be an HTTP loopback address")
    require(not url.username and not url.password and url.path in ("", "/")
            and not url.query and not url.fragment, "base URL must contain only host and port")
    require(url.hostname == "localhost" or ipaddress.ip_address(url.hostname).is_loopback,
            "base URL must use localhost or a loopback IP address")
    return url.hostname, url.port or 80


def exchange(address, method, path, payload=None):
    """Observe one attempt; only a dropped response or short body is retryable.

    Connect/write failures, timeouts, invalid HTTP, chunking, oversized bodies,
    and mismatched fixture bytes fail the example instead of masquerading as a
    deliberately injected fault. Each retry gets a new connection.
    """
    connection = http.client.HTTPConnection(*address, timeout=TIMEOUT_SECONDS)
    started = time.monotonic()
    try:
        connection.request(method, path, json.dumps(payload) if payload is not None else None,
                           {"Content-Type": "application/json"} if payload is not None else {})
        try:
            response = connection.getresponse()
        except (http.client.RemoteDisconnected, ConnectionResetError) as error:
            return {"kind": "connection_drop", "status": None, "headers": {}, "body": b"",
                    "exception": type(error).__name__,
                    "elapsed_ms": round((time.monotonic() - started) * 1000, 3)}
        headers = {key.lower(): value for key, value in response.getheaders()}
        lengths = response.headers.get_all("Content-Length", [])
        require("transfer-encoding" not in headers, "example expects bounded Content-Length responses")
        require(len(lengths) == 1 or (response.status == 204 and not lengths),
                "response must have exactly one Content-Length")
        length = int(lengths[0]) if lengths else 0
        require(0 <= length <= MAX_BODY_BYTES, "response exceeds 64 KiB or has invalid length")
        # An unbounded read is safe here only because chunking is rejected and
        # the declared length is validated first. It detects incomplete framing.
        try:
            body = response.read()
            kind = "response"
        except http.client.IncompleteRead as error:
            body = error.partial
            kind = "truncated_response"
        require(len(body) <= MAX_BODY_BYTES, "response exceeds 64 KiB")
        return {"kind": kind, "status": response.status, "headers": headers, "body": body,
                "declared_body_bytes": length,
                "elapsed_ms": round((time.monotonic() - started) * 1000, 3)}
    finally:
        connection.close()


def management(address, method, path, payload=None, status=200):
    response = exchange(address, method, path, payload)
    require(response["kind"] == "response" and response["status"] == status,
            f"{method} {path}: expected complete HTTP {status}, got {response['kind']} {response['status']}")
    if status == 204:
        require(response["body"] == b"", "DELETE returned a body")
        return None
    result = json.loads(response["body"])
    require(isinstance(result, dict), f"{method} {path}: expected a JSON object")
    return result


def fixture(body, status=200, fault=None):
    step = {"status_code": status, "duration_ms": 10, "method": "tokio_sleep",
            "fixture": {"body": body, "content_type": "application/json; charset=utf-8",
                        "headers": {"retry-after": "0", "x-request-id": "retry-client"}}}
    if fault is not None:
        step["fault"] = fault
    return step


def cases():
    return [
        ("status_recovery", [
            fixture('{ "ready": false, "attempt": 1 }\n', 503),
            fixture('{ "ready": false, "attempt": 2 }\n', 503),
            fixture('{ "ready": true, "message": "café" }\n'),
        ]),
        ("transport_recovery", [
            {"status_code": 503, "duration_ms": 10, "method": "tokio_sleep",
             "fault": {"type": "connection_drop"}},
            fixture('{ "ready": true, "message": "café" }\n',
                    fault={"type": "truncated_response", "after_bytes": 12}),
            fixture('{ "ready": true, "message": "recovered" }\n'),
        ]),
    ]


def retry_sequence(address, path, generation, sequence, observations):
    """Retry only the fixture's 503s and explicit transport failures, at most 3 times."""
    for number in range(1, MAX_ATTEMPTS + 1):
        result = exchange(address, "POST", path + "/attempts", {"generation": generation})
        step = sequence[number - 1]
        expected_kind = step.get("fault", {}).get("type", "response")
        evidence = {"attempt": number, "kind": result["kind"], "http_status": result["status"],
                    "body_hex": result["body"].hex(), "elapsed_ms": result["elapsed_ms"]}
        observations.append(evidence)
        require(result["kind"] == expected_kind,
                f"attempt {number}: expected {expected_kind}, got {result['kind']}")
        if expected_kind == "connection_drop":
            evidence["exception"] = result["exception"]
            delay = FAULT_BACKOFF_SECONDS
        else:
            require(result["status"] == step["status_code"], f"attempt {number}: wrong HTTP status")
            expected_body = step["fixture"]["body"].encode("utf-8")
            require(result["declared_body_bytes"] == len(expected_body), "incorrect Content-Length")
            require(result["headers"].get("content-type") == step["fixture"]["content_type"],
                    "fixture content type differs")
            for name, value in step["fixture"]["headers"].items():
                require(result["headers"].get(name) == value, f"fixture header {name} differs")
            if expected_kind == "truncated_response":
                expected_body = expected_body[:step["fault"]["after_bytes"]]
            require(result["body"] == expected_body, f"attempt {number}: fixture bytes differ")
            evidence["fixture_bytes_match"] = True
            evidence["declared_body_bytes"] = result["declared_body_bytes"]
            if expected_kind == "response" and result["status"] == 200:
                require(json.loads(result["body"])["ready"] is True, "consumer did not parse ready=true")
                require(number == len(sequence), "consumer recovered before the final step")
                evidence["consumer_ready"] = True
                return
            require(expected_kind == "truncated_response" or result["status"] == 503,
                    "retry policy rejects this response")
            delay = (FAULT_BACKOFF_SECONDS if expected_kind == "truncated_response"
                     else int(result["headers"]["retry-after"]))
        require(number < MAX_ATTEMPTS, "retry budget exhausted without recovery")
        evidence["retry_delay_ms"] = round(delay * 1000)
        time.sleep(delay)
    raise AssertionError("retry budget exhausted without recovery")


def receipt_snapshot(address, path, run_id, generation, sequence):
    snapshot = management(address, "GET", path + "/receipts")
    require(snapshot.get("run_id") == run_id and snapshot.get("generation") == generation,
            "receipt run/generation differs")
    receipts = snapshot.get("receipts", [])
    require(len(receipts) == len(sequence), "receipt count differs from client attempts")
    for number, (receipt, step) in enumerate(zip(receipts, sequence), 1):
        require(receipt.get("generation") == generation and receipt.get("attempt") == number
                and receipt.get("sequence_step") == number, "receipt correlation differs")
        require(receipt.get("selected_status_code") == step["status_code"]
                and receipt.get("method") == step["method"]
                and receipt.get("requested_duration_ms") == step["duration_ms"],
                "receipt selection differs")
        fault = step.get("fault")
        require(receipt.get("outcome") == ("fault_injected" if fault else "completed"),
                "unexpected server receipt outcome")
        if fault:
            require(receipt.get("fault") == fault, "receipt fault differs")
            expected_stage = "before_headers" if fault["type"] == "connection_drop" else "after_body_prefix"
            require(receipt.get("injection_stage") == expected_stage
                    and receipt.get("injected_body_bytes") == fault.get("after_bytes", 0),
                    "server injection metadata differs")
    return snapshot


def exercise(address, report):
    require(management(address, "GET", "/health") == {"status": "ok"}, "health check failed")
    for name, sequence in cases():
        case = {"name": name, "passed": False, "generations": [], "cleanup": "not_created"}
        report["cases"].append(case)
        run = management(address, "POST", "/runs", {"sequence": sequence, "ttl_ms": 60000}, 201)
        run_id = run.get("run_id", "")
        require(re.fullmatch(r"run-\d+", run_id) is not None, "invalid generated run ID")
        path = "/runs/" + run_id
        case["run_id"] = run_id
        case["cleanup"] = "pending"
        try:
            generation = run["generation"]
            for replay in range(2):
                record = {"generation": generation, "client_attempts": []}
                case["generations"].append(record)
                retry_sequence(address, path, generation, sequence, record["client_attempts"])
                record["server_receipts"] = receipt_snapshot(address, path, run_id, generation, sequence)
                exhausted = management(address, "POST", path + "/attempts", {"generation": generation}, 409)
                require(exhausted.get("simulated") is False, "exhaustion must be an API error")
                if replay == 0:
                    reset = management(address, "POST", path + "/reset", {"generation": generation})
                    require(reset["generation"] == generation + 1 and reset["generation_attempts"] == 0
                            and reset["retained_receipts"] == 0 and reset["attempts_started"] == 3,
                            "reset did not rewind and clear receipts while preserving lifetime attempts")
                    stale = management(address, "POST", path + "/attempts", {"generation": generation}, 409)
                    require(stale.get("simulated") is False, "stale generation must be an API error")
                    require(management(address, "GET", path)["generation_attempts"] == 0,
                            "stale generation consumed a sequence step")
                    require(management(address, "GET", path + "/receipts")["receipts"] == [],
                            "reset/stale request left receipts behind")
                    generation = reset["generation"]
            final = management(address, "GET", path)
            require(final["attempts_started"] == 6 and final["generation_attempts"] == 3,
                    "replay or rejected requests changed the lifetime attempt budget")
            case["reset_replay_passed"] = True
            case["passed"] = True
        finally:
            management(address, "DELETE", path, status=204)
            management(address, "GET", path, status=404)
            case["cleanup"] = "deleted_and_absent"


class Server:
    """Own one child, continuously drain logs, and stop it on every exit path."""

    def __init__(self, binary):
        self.binary = binary
        self.child = None
        self.reader = None
        self.logs = deque(maxlen=20)
        self.startup = queue.Queue(maxsize=1)

    def __enter__(self):
        env = dict(os.environ)
        env.update({"SLEEPAPI_ADDR": "127.0.0.1:0", "SLEEPAPI_MAX_DURATION_MS": "1000",
                    "SLEEPAPI_MAX_IN_FLIGHT": "64", "SLEEPAPI_SHUTDOWN_GRACE_MS": "500",
                    "SLEEPAPI_CORS_ORIGINS": "", "TOKIO_WORKER_THREADS": "2", "RUST_LOG": "sleepapi=info"})
        self.child = subprocess.Popen([str(self.binary)], env=env, stdout=subprocess.PIPE,
                                      stderr=subprocess.STDOUT, text=True)

        def drain():
            for line in self.child.stdout:
                self.logs.append(line.rstrip())
                if "sleepapi listening" in line:
                    match = re.search(r"127\.0\.0\.1:(\d+)", line)
                    if match:
                        try:
                            self.startup.put_nowait(int(match.group(1)))
                        except queue.Full:
                            pass

        self.reader = threading.Thread(target=drain, daemon=True)
        self.reader.start()
        try:
            return "127.0.0.1", self.startup.get(timeout=5)
        except BaseException:
            self.cleanup()
            raise RuntimeError("server did not become ready: " + "\n".join(self.logs))

    def cleanup(self):
        if self.child is not None and self.child.poll() is None:
            self.child.terminate()
            try:
                self.child.wait(timeout=2)
            except subprocess.TimeoutExpired:
                self.child.kill()
                self.child.wait(timeout=2)
        if self.reader is not None:
            self.reader.join(timeout=2)
        if self.child is not None and self.child.stdout is not None:
            self.child.stdout.close()

    def __exit__(self, error_type, _error, _traceback):
        self.cleanup()
        if error_type is None:
            require(self.child.returncode == 0, f"owned server exited {self.child.returncode}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    target = parser.add_mutually_exclusive_group()
    target.add_argument("--base-url", default="http://127.0.0.1:3000",
                        help="existing local HTTP server (default: %(default)s)")
    target.add_argument("--binary", type=Path, help="launch and stop this SleepAPI binary on loopback")
    parser.add_argument("--output", type=Path, help="also save the JSON report to a new file")
    args = parser.parse_args()
    report = {"schema_version": 1, "target_release": TARGET_RELEASE,
              "server_version_verified": False, "observed_at": datetime.now(timezone.utc).isoformat(),
              "scope": "synthetic retry-client example; not external client acceptance",
              "passed": False, "policy": {"max_attempts": MAX_ATTEMPTS,
              "request_timeout_seconds": TIMEOUT_SECONDS, "max_response_body_bytes": MAX_BODY_BYTES,
              "fault_backoff_seconds": FAULT_BACKOFF_SECONDS}, "cases": []}
    output = None
    try:
        if args.output is not None:
            output = args.output.open("x", encoding="utf-8")
        if args.binary is not None:
            binary = args.binary.resolve(strict=True)
            report["binary_sha256"] = hashlib.sha256(binary.read_bytes()).hexdigest()
            with Server(binary) as address:
                exercise(address, report)
            report["owned_server_exit_code"] = 0
        else:
            exercise(local_address(args.base_url), report)
        report["passed"] = True
    except (OSError, ValueError, RuntimeError, AssertionError, KeyError,
            http.client.HTTPException, subprocess.SubprocessError) as error:
        report["error"] = f"{type(error).__name__}: {error}"
    finally:
        encoded = json.dumps(report, indent=2, allow_nan=False) + "\n"
        if output is not None:
            with output:
                output.write(encoded)
        print(encoded, end="")
    return 0 if report["passed"] else 1


if __name__ == "__main__":
    sys.exit(main())

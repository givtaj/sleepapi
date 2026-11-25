#!/usr/bin/env python3
"""Verify how a synthetic worker recovers from a lost provider reply.

Pass both comparison binaries, or verify an installed executable on its own.
The worker is a separate process in each phase. Raw HTTP/1.1 reads count status
line, header, and body bytes together. All endpoints are owned loopback servers.
"""

import argparse
from concurrent.futures import ThreadPoolExecutor
from datetime import datetime, timezone
import hashlib
import http.client
import json
import os
from pathlib import Path
import re
import runpy
import socket
import subprocess
import sys
import tempfile
import time


ROOT = Path(__file__).resolve().parents[1]
RETRY_EXAMPLE = runpy.run_path(str(ROOT / "examples/retry-client.py"))
Server = RETRY_EXAMPLE["Server"]
management = RETRY_EXAMPLE["management"]
require = RETRY_EXAMPLE["require"]
HOST = "127.0.0.1"
SOCKET_TIMEOUT = 2
MAX_RESPONSE_BYTES = 65536
POLLS = 3
SYNTHETIC_INTENT = "synthetic-intent"
ACCEPTANCE_FIXTURE = json.dumps({"intent_id": SYNTHETIC_INTENT, "accepted": True})


def attempt_request(port, run_id, generation):
    payload = json.dumps({"generation": generation}).encode("ascii")
    return (
        f"POST /runs/{run_id}/attempts HTTP/1.1\r\n"
        f"Host: {HOST}:{port}\r\nConnection: close\r\n"
        f"Content-Type: application/json\r\nContent-Length: {len(payload)}\r\n\r\n"
    ).encode("ascii") + payload


def exchange_wire(port, request):
    """Require a successful write, then read through EOF or reset."""
    with socket.create_connection((HOST, port), timeout=SOCKET_TIMEOUT) as peer:
        peer.sendall(request)
        deadline = time.monotonic() + SOCKET_TIMEOUT
        received = bytearray()
        while True:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise TimeoutError("HTTP response read deadline expired")
            peer.settimeout(remaining)
            try:
                part = peer.recv(4096)
            except ConnectionResetError:
                return bytes(received), "reset"
            if not part:
                return bytes(received), "eof"
            received += part
            require(len(received) <= MAX_RESPONSE_BYTES, "HTTP response exceeds 64 KiB")


def observe(port, request, expected_body=None):
    wire, ending = exchange_wire(port, request)
    if not wire:
        return {"response_bytes": 0, "http_status": None, "connection_end": ending}
    head, separator, body = wire.partition(b"\r\n\r\n")
    lines = head.split(b"\r\n")
    status = re.fullmatch(rb"HTTP/1\.1 (\d{3}) .+", lines[0])
    lengths = [line.split(b":", 1)[1].strip() for line in lines[1:]
               if line.lower().startswith(b"content-length:")]
    require(ending == "eof" and separator and status is not None and
            len(lengths) == 1 and lengths[0] == str(len(body)).encode("ascii") and
            not any(line.lower().startswith(b"transfer-encoding:") for line in lines[1:]),
            "expected a complete HTTP/1.1 response with a matching Content-Length")
    if expected_body is not None:
        require(body == expected_body, "complete response did not match the acceptance fixture")
    return {"response_bytes": len(wire), "http_status": int(status.group(1)),
            "body_bytes": len(body), "connection_end": ending}


def matching_receipt(snapshot, run_id, generation, attempt):
    require(snapshot["run_id"] == run_id and snapshot["generation"] == generation,
            "receipt lookup returned a different run or generation")
    matches = [item for item in snapshot["receipts"]
               if item["generation"] == generation and item["attempt"] == attempt
               and item["sequence_step"] == attempt]
    require(len(matches) == 1, "expected exactly one correlated attempt receipt")
    return matches[0]


def accepted_before_loss(receipt, run_id, generation, attempt, proxy_record):
    require(receipt["selected_status_code"] == 200, "ambiguous step must select 200")
    if proxy_record is not None:
        proxy = json.loads(proxy_record.read_text(encoding="utf-8"))
        require((proxy["run_id"], proxy["generation"], proxy["attempt"]) ==
                (run_id, generation, attempt)
                and proxy["provider_accepted"] is True
                and receipt["outcome"] == "completed",
                "proxy acceptance record and SleepAPI receipt do not agree")
        return True, "proxy_provider_record"
    require(receipt["outcome"] == "fault_injected" and
            receipt["injection_stage"] == "before_headers" and
            receipt["injected_body_bytes"] == 0, "missing pre-header drop evidence")
    if receipt["fault"] == {"type": "connection_drop"} and "provider_accepted" not in receipt:
        # The released receipt has no acceptance field. Only this controlled
        # synthetic fixture permits replay when acceptance is unknown.
        return None, "synthetic_fixture_retry_policy"
    accepted = receipt.get("provider_accepted")
    require(isinstance(accepted, bool), "missing explicit provider acceptance")
    return accepted, "sleepapi_receipt"


def worker(phase, port, run_id, generation, state_path, proxy_record):
    """Persist ambiguous intent in one process and reconcile it after restart."""
    state_path = Path(state_path)
    proxy_record = Path(proxy_record) if proxy_record else None
    intent = {"intent_id": SYNTHETIC_INTENT, "run_id": run_id, "generation": generation,
              "attempt": 1, "status": "ambiguous"}
    request = attempt_request(port, run_id, generation)
    if phase == "initial":
        with state_path.open("x", encoding="utf-8") as state_file:
            json.dump(intent, state_file)
        send = observe(port, request)
        require(send["response_bytes"] == 0, "worker received bytes on its initial send")
        return {"pid": os.getpid(), "send": send}

    stored = json.loads(state_path.read_text(encoding="utf-8"))
    require(stored == intent, "restart did not load the original ambiguous intent")
    polls = []
    for number in range(1, POLLS + 1):
        poll = {"number": number, "decision": "already_resolved"}
        if stored["status"] == "ambiguous":
            snapshot = management((HOST, port), "GET", f"/runs/{run_id}/receipts")
            receipt = matching_receipt(snapshot, run_id, generation, stored["attempt"])
            accepted, source = accepted_before_loss(
                receipt, run_id, generation, stored["attempt"], proxy_record)
            poll["provider_accepted"] = accepted
            poll["acceptance_source"] = source
            poll["decision"] = "deduplicate" if accepted else "retry"
            if not accepted:
                poll["send"] = observe(port, request)
                require(poll["send"]["http_status"] == 200, "retry did not return a complete 200")
            stored["status"] = "resolved"
            state_path.write_text(json.dumps(stored), encoding="utf-8")
        polls.append(poll)
        if number != POLLS:
            time.sleep(0.025)
    return {"pid": os.getpid(), "loaded_intent": intent, "polls": polls}


def launch_worker(phase, port, run_id, generation, state, proxy_record=None):
    command = [sys.executable, str(Path(__file__).resolve()), "--worker",
               phase, str(port), run_id, str(generation), str(state),
               str(proxy_record) if proxy_record else ""]
    child = subprocess.run(command, capture_output=True, encoding="utf-8", timeout=10)
    require(child.returncode == 0, f"{phase} worker exited {child.returncode}: {child.stderr.strip()}")
    return json.loads(child.stdout)


def swallow_reply(listener, upstream_port, expected, run_id, generation, record_path):
    downstream, _ = listener.accept()
    with downstream:
        deadline = time.monotonic() + SOCKET_TIMEOUT
        request = bytearray()
        while len(request) < len(expected):
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise TimeoutError("proxy request read deadline expired")
            downstream.settimeout(remaining)
            part = downstream.recv(len(expected) - len(request))
            if not part:
                break
            request += part
        require(request == expected, "proxy received an unexpected synthetic request")
        received = observe(upstream_port, request, expected_body=ACCEPTANCE_FIXTURE.encode("utf-8"))
        require(received["http_status"] == 200,
                "proxy did not consume a complete provider 200")
        record = {"run_id": run_id, "generation": generation, "attempt": 1,
                  "provider_accepted": True, "verified_fixture": json.loads(ACCEPTANCE_FIXTURE),
                  "upstream": received, "forwarded_bytes": 0}
        record_path.write_text(json.dumps(record), encoding="utf-8")
        # Closing downstream delivers no bytes to the worker.
        return record


def run_case(address, directory, name, fault, explicit_acceptance, expect_retry):
    step = {"status_code": 200, "duration_ms": 0}
    first = dict(step)
    if fault is None:
        first["fixture"] = {"body": ACCEPTANCE_FIXTURE}
    else:
        first["fault"] = {"type": fault}
    run = management(address, "POST", "/runs", {"sequence": [first, step]}, status=201)
    run_id, generation = run["run_id"], run["generation"]
    require(re.fullmatch(r"run-\d+", run_id) is not None, "invalid generated run ID")
    path = f"/runs/{run_id}"
    state = directory / f"{name}-worker.json"
    proxy_record = None
    result = {"name": name, "run_id": run_id, "generation": generation, "sequence": [first, step]}
    try:
        if fault is None:
            proxy_record = directory / f"{name}-proxy.json"
            with socket.create_server((HOST, 0)) as listener, ThreadPoolExecutor(max_workers=1) as pool:
                listener.settimeout(SOCKET_TIMEOUT)
                port = listener.getsockname()[1]
                expected = attempt_request(port, run_id, generation)
                proxy_task = pool.submit(
                    swallow_reply, listener, address[1], expected, run_id, generation, proxy_record)
                result["initial_worker"] = launch_worker("initial", port, run_id, generation, state)
                result["proxy_record"] = proxy_task.result(timeout=SOCKET_TIMEOUT)
        else:
            result["initial_worker"] = launch_worker("initial", address[1], run_id, generation, state)

        before = management(address, "GET", path + "/receipts")
        initial_receipt = matching_receipt(before, run_id, generation, 1)
        require(len(before["receipts"]) == 1, "unexpected number of first-worker attempts")
        if fault is None:
            require(initial_receipt["outcome"] == "completed" and
                    all(field not in initial_receipt for field in ("fault", "injection_stage", "injected_body_bytes")),
                    "proxy case must leave an ordinary completed SleepAPI receipt")
        else:
            require(initial_receipt["fault"] == first["fault"], "wrong fault in initial receipt")
        if explicit_acceptance is None:
            require("provider_accepted" not in initial_receipt,
                    "this comparison expected no native provider acceptance field")
        else:
            require(initial_receipt.get("provider_accepted") is explicit_acceptance,
                    "first provider receipt acceptance differs")
        result["receipts_before_restart"] = before
        result["worker_state_before_restart"] = json.loads(state.read_text(encoding="utf-8"))

        restarted = launch_worker("restart", address[1], run_id, generation, state, proxy_record)
        require(restarted["pid"] != result["initial_worker"]["pid"], "worker process did not restart")
        decisions = ["retry" if expect_retry else "deduplicate"] + ["already_resolved"] * (POLLS - 1)
        require([poll["decision"] for poll in restarted["polls"]] == decisions,
                "post-restart worker decisions differ")
        require(sum("send" in poll for poll in restarted["polls"]) == int(expect_retry),
                "post-restart worker sends differ")
        result["restarted_worker"] = restarted
        result["worker_state_after_restart"] = json.loads(state.read_text(encoding="utf-8"))
        require(result["worker_state_after_restart"]["status"] == "resolved",
                "worker did not persist resolution")

        after = management(address, "GET", path + "/receipts")
        require(len(after["receipts"]) == 1 + int(expect_retry)
                and matching_receipt(after, run_id, generation, 1) == initial_receipt,
                "original provider receipt changed or an unexpected send occurred")
        if expect_retry:
            retry = matching_receipt(after, run_id, generation, 2)
            require(retry["outcome"] == "completed" and retry["selected_status_code"] == 200,
                    "provider did not complete the second send")
        result["receipts_after_restart"] = after
    finally:
        management(address, "DELETE", path, status=204)
        management(address, "GET", path, status=404)
    result["cleanup"] = "deleted_and_absent"
    return result


def verify_binary(binary, directory, role, result):
    binary = binary.resolve(strict=True)
    result["binary_sha256"] = hashlib.sha256(binary.read_bytes()).hexdigest()
    with Server(binary) as address:
        metadata = management(address, "GET", "/metadata")
        capabilities = metadata["capabilities"]
        result["software"] = metadata["software"]
        require(capabilities["connection_drop"] is True, "listener lacks connection_drop")
        result["capabilities"] = {
            "connection_drop": True, "accept_then_drop": capabilities.get("accept_then_drop"),
        }
        result["cases"] = cases = []
        if role == "alpha5":
            require(metadata["software"]["version"] == "0.1.1-alpha.5"
                    and "accept_then_drop" not in capabilities, "expected alpha.5 metadata")
            rejection = management(address, "POST", "/runs", {"sequence": [
                {"status_code": 200, "duration_ms": 0, "fault": {"type": "accept_then_drop"}},
            ]}, status=422)
            require("unknown variant `accept_then_drop`" in rejection["error"]["message"],
                    "alpha.5 rejected the configuration for an unexpected reason")
            require(rejection["simulated"] is False
                    and management(address, "GET", "/runs")["active_runs"] == 0,
                    "alpha.5 rejection created a run or returned a simulated response")
            result["native_configuration_rejection"] = {"http_status": 422, "body": rejection}
            cases.append(run_case(address, directory, "alpha5_plain_drop", "connection_drop", None, True))
            cases.append(run_case(address, directory, "alpha5_proxy_swallow", None, None, False))
        else:
            require(capabilities.get("accept_then_drop") is True, f"{role} lacks accept_then_drop")
            cases.append(run_case(address, directory, f"{role}_plain_drop", "connection_drop", False, True))
            cases.append(run_case(address, directory, f"{role}_accept_then_drop", "accept_then_drop", True, False))
    result["owned_server_exit_code"] = 0


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--alpha5-binary", type=Path, help="published alpha.5 executable for comparison")
    parser.add_argument("--candidate-binary", type=Path, help="local candidate executable for comparison")
    parser.add_argument("--installed-binary", type=Path, help="installed executable to verify on its own")
    args = parser.parse_args()
    if args.installed_binary is not None:
        if args.alpha5_binary is not None or args.candidate_binary is not None:
            parser.error("--installed-binary must be used on its own")
        binaries = (("installed", args.installed_binary),)
    else:
        if args.alpha5_binary is None or args.candidate_binary is None:
            parser.error("provide --installed-binary or both --alpha5-binary and --candidate-binary")
        binaries = (("alpha5", args.alpha5_binary), ("candidate", args.candidate_binary))
    report = {"schema_version": 1, "scope": "synthetic worker on loopback", "passed": False,
              "observed_at": datetime.now(timezone.utc).isoformat()}
    try:
        with tempfile.TemporaryDirectory(prefix="sleepapi-lost-reply-") as directory:
            for role, binary in binaries:
                report[role] = {}
                verify_binary(binary, Path(directory), role, report[role])
        report["passed"] = True
    except (AssertionError, OSError, ValueError, KeyError, RuntimeError,
            http.client.HTTPException, subprocess.SubprocessError) as error:
        report["error"] = f"{type(error).__name__}: {error}"
    print(json.dumps(report, indent=2, allow_nan=False))
    return 0 if report["passed"] else 1


if __name__ == "__main__":
    if sys.argv[1:2] == ["--worker"]:
        _, _, phase, port, run_id, generation, state, proxy_record = sys.argv
        print(json.dumps(worker(phase, int(port), run_id, int(generation), state, proxy_record)))
    else:
        sys.exit(main())

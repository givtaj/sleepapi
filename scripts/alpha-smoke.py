#!/usr/bin/env python3
"""Check a local SleepAPI binary against the alpha HTTP regression thresholds.

Build first with `cargo build --release --locked`, then run this script. It uses
only Python's standard library and launches its own loopback-only processes.
The fixed workload is a local regression check, not a production capacity or
tail-latency certification. Linux and macOS are supported; Windows is unverified.
"""

import argparse
from collections import deque
from concurrent.futures import ThreadPoolExecutor
from datetime import datetime, timezone
import hashlib
import http.client
import json
import math
import os
from pathlib import Path
import platform
import queue
import re
import subprocess
import sys
import threading
import time


ROUTES = {
    "tokio": "tokio_sleep",
    "tokio-spawn": "tokio_spawn",
    "thread": "thread_sleep",
    "park": "thread_park",
}
REQUEST_COUNT = 64
CONCURRENCY = 16
DURATION_MS = 40
OVERHEAD_LIMIT_MS = 250
HEALTH_LIMIT_MS = 250
CAPACITY_WAIT_MS = 300
RECOVERY_ALLOWANCE_MS = 1000
REQUEST_TIMEOUT_SECS = 2


def require(condition, message):
    if not condition:
        raise AssertionError(message)


def request(port, path, payload=None):
    connection = http.client.HTTPConnection(
        "127.0.0.1", port, timeout=REQUEST_TIMEOUT_SECS
    )
    started = time.monotonic()
    try:
        headers = {"Content-Type": "application/json"} if payload is not None else {}
        connection.request(
            "POST" if payload is not None else "GET", path,
            json.dumps(payload) if payload is not None else None, headers,
        )
        response = connection.getresponse()
        raw = response.read(65537)
        require(len(raw) <= 65536, "response exceeded 64 KiB")
        body = json.loads(raw)
        require(isinstance(body, dict), "response must be a JSON object")
        return response.status, body, (time.monotonic() - started) * 1000
    finally:
        connection.close()


def simulation(port, route, duration_ms):
    return validate_simulation(
        *request(port, "/sleep/" + route, {"duration_ms": duration_ms}), route, duration_ms
    )


def validate_simulation(status, body, elapsed_ms, route, duration_ms):
    require(status == 200 and body.get("status_code") == 200,
            f"{route}: expected HTTP/body status 200, got {status}: {body}")
    require(body.get("simulated") is True and body.get("operation") == "sleep",
            f"{route}: not a simulated sleep result")
    require(body.get("method") == ROUTES[route], f"{route}: incorrect waiting method")
    require(body.get("blocks_thread") is (route in ("thread", "park")),
            f"{route}: incorrect blocks_thread value")
    require(body.get("requested_duration_ms") == duration_ms,
            f"{route}: incorrect requested duration")
    server_elapsed = body.get("elapsed_ms")
    require(type(server_elapsed) in (int, float) and math.isfinite(server_elapsed)
            and server_elapsed >= duration_ms, f"{route}: invalid/early server timing")
    require(elapsed_ms >= duration_ms, f"{route}: response arrived before requested wait")
    require("error" not in body, f"{route}: success contained an error")
    return elapsed_ms


def health(port):
    status, body, elapsed_ms = request(port, "/health")
    require(status == 200 and body == {"status": "ok"}, "health contract failed")
    require(elapsed_ms <= HEALTH_LIMIT_MS,
            f"health took {elapsed_ms:.1f} ms; limit is {HEALTH_LIMIT_MS} ms")
    return elapsed_ms


class Server:
    """Own exactly one child; drain logs without retaining unbounded output."""

    def __init__(self, binary, capacity):
        self.binary = binary
        self.capacity = capacity
        self.child = None
        self.logs = deque(maxlen=30)
        self.startup = queue.Queue(maxsize=1)
        self.reader = None

    def __enter__(self):
        env = dict(os.environ)
        env.update({
            "SLEEPAPI_ADDR": "127.0.0.1:0",
            "SLEEPAPI_MAX_DURATION_MS": "10000",
            "SLEEPAPI_MAX_IN_FLIGHT": str(self.capacity),
            "SLEEPAPI_SHUTDOWN_GRACE_MS": "500",
            "SLEEPAPI_CORS_ORIGINS": "",
            "TOKIO_WORKER_THREADS": "2",
            "RUST_LOG": "sleepapi=info",
        })
        self.child = subprocess.Popen(
            [str(self.binary)], env=env, stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT, text=True,
        )

        def read_logs():
            for line in self.child.stdout:
                self.logs.append(line.rstrip())
                if "sleepapi listening" in line:
                    match = re.search(r"127\.0\.0\.1:(\d+)", line)
                    if match:
                        try:
                            self.startup.put_nowait(int(match.group(1)))
                        except queue.Full:
                            pass

        self.reader = threading.Thread(target=read_logs, daemon=True)
        self.reader.start()
        try:
            self.port = self.startup.get(timeout=5)
            require(1 <= self.port <= 65535, "invalid loopback port in startup log")
            health(self.port)
            return self
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

    def __exit__(self, exception_type, _exception, _traceback):
        self.cleanup()
        if exception_type is None:
            require(self.child.returncode == 0,
                    f"graceful shutdown exited {self.child.returncode}: " + "\n".join(self.logs))


def check_route(port, route):
    health_samples = []
    with ThreadPoolExecutor(max_workers=CONCURRENCY) as workers:
        pending = [workers.submit(simulation, port, route, DURATION_MS)
                   for _ in range(REQUEST_COUNT)]
        # Probe while the submitted workload is outstanding. The capacity-one
        # check below additionally proves health while a simulation is admitted.
        while not all(future.done() for future in pending):
            health_samples.append(health(port))
            time.sleep(0.01)
        latencies = [future.result() for future in pending]
    require(len(health_samples) >= 2, f"{route}: fewer than two health samples during workload")
    p95_ms = sorted(latencies)[math.ceil(0.95 * len(latencies)) - 1]
    overhead_ms = p95_ms - DURATION_MS
    require(overhead_ms <= OVERHEAD_LIMIT_MS,
            f"{route}: p95 overhead {overhead_ms:.1f} ms exceeds {OVERHEAD_LIMIT_MS} ms")
    return {
        "route": "/sleep/" + route, "passed": True,
        "validated_responses": len(latencies), "unexpected_responses": 0,
        "p95_latency_ms": round(p95_ms, 3),
        "p95_overhead_ms": round(overhead_ms, 3),
        "health_samples": len(health_samples),
        "max_health_ms": round(max(health_samples), 3),
    }


def check_capacity(port, route):
    deadline = time.monotonic() + 2
    attempts = 0
    with ThreadPoolExecutor(max_workers=1) as worker:
        occupied = False
        while time.monotonic() < deadline:
            attempts += 1
            started = time.monotonic()
            waiting = worker.submit(request, port, "/sleep/" + route,
                                    {"duration_ms": CAPACITY_WAIT_MS})
            while time.monotonic() < deadline and not waiting.done():
                status, body, elapsed = request(port, "/sleep/tokio", {"duration_ms": 0})
                if status == 429:
                    validate_capacity_rejection(status, body)
                    occupied = True
                    break
                validate_simulation(status, body, elapsed, "tokio", 0)
                time.sleep(0.005)
            if occupied:
                break
            # A zero-delay probe can win the slot and reject the long request.
            # Consume and validate that result before retrying, as in the Rust
            # process tests; never assume a guessed startup sleep means admission.
            result = waiting.result(timeout=REQUEST_TIMEOUT_SECS)
            if result[0] == 429:
                validate_capacity_rejection(result[0], result[1])
            else:
                validate_simulation(*result, route, CAPACITY_WAIT_MS)
        require(occupied, f"{route}: never proved the long request occupied its slot")
        require(not waiting.done(), f"{route}: wait finished before the occupied health check")
        health_ms = health(port)
        require(not waiting.done(), f"{route}: wait finished during the occupied health check")
        validate_simulation(*waiting.result(timeout=REQUEST_TIMEOUT_SECS), route, CAPACITY_WAIT_MS)
        # A real simulation, rather than health, proves admission has recovered.
        simulation(port, route, 0)
    recovered_ms = (time.monotonic() - started) * 1000
    require(recovered_ms <= CAPACITY_WAIT_MS + RECOVERY_ALLOWANCE_MS,
            f"{route}: recovery took {recovered_ms:.1f} ms")
    return {
        "route": "/sleep/" + route, "passed": True,
        "admission_attempts": attempts,
        "capacity_exceeded_observed": True, "admission_recovered": True,
        "recovery_from_request_launch_ms": round(recovered_ms, 3),
        "health_while_occupied_ms": round(health_ms, 3),
    }


def validate_capacity_rejection(status, body):
    require(status == 429 and body.get("simulated") is False and body.get("status_code") == 429
            and isinstance(body.get("error"), dict)
            and body["error"].get("code") == "capacity_exceeded",
            f"invalid capacity rejection: {body}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=Path("target/release/sleepapi"))
    parser.add_argument("--output", type=Path,
                        help="new JSON evidence file; existing files are never overwritten")
    args = parser.parse_args()
    if os.name != "posix":
        parser.error("this alpha gate supports Linux/macOS; Windows is unverified")
    binary = args.binary.resolve()
    if not binary.is_file():
        parser.error(f"binary does not exist: {binary}; run cargo build --release --locked")
    output = args.output.open("x", encoding="utf-8") if args.output else None
    root = Path(__file__).resolve().parent.parent
    source_paths = [root / name for name in ("Cargo.toml", "Cargo.lock", "Dockerfile")]
    source_paths.extend(sorted((root / "src").glob("*.rs")))
    source_manifest = {str(path.relative_to(root)): hashlib.sha256(path.read_bytes()).hexdigest()
                       for path in source_paths}
    report = {
        "schema_version": 1, "passed": False,
        "checked_at_utc": datetime.now(timezone.utc).isoformat(),
        "binary": str(binary), "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
        "harness_sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
        "provenance_note": "Source snapshot and binary hash; retain the preceding build/CI result to establish their association.",
        "source_file_sha256": source_manifest,
        "source_manifest_sha256": hashlib.sha256(
            json.dumps(source_manifest, sort_keys=True, separators=(",", ":")).encode()
        ).hexdigest(),
        "platform": platform.platform(), "python_version": platform.python_version(),
        "scope": "local loopback regression smoke; not a capacity certification",
        "workload": {"requests_per_route": REQUEST_COUNT, "concurrency": CONCURRENCY,
                     "duration_ms": DURATION_MS, "server_capacity": 64,
                     "tokio_worker_threads": 2, "request_timeout_secs": REQUEST_TIMEOUT_SECS},
        "thresholds": {"unexpected_responses": 0, "p95_overhead_ms": OVERHEAD_LIMIT_MS,
                       "health_max_ms": HEALTH_LIMIT_MS,
                       "capacity_wait_ms": CAPACITY_WAIT_MS,
                       "capacity_recovery_allowance_ms": RECOVERY_ALLOWANCE_MS,
                       "graceful_exit_code": 0},
        "routes": [], "capacity": [],
    }
    try:
        with Server(binary, 64) as server:
            for route in ROUTES:
                report["routes"].append(check_route(server.port, route))
        report["workload_server_graceful_exit"] = 0
        with Server(binary, 1) as server:
            for route in ROUTES:
                report["capacity"].append(check_capacity(server.port, route))
        report["capacity_server_graceful_exit"] = 0
        report["passed"] = True
    except (OSError, ValueError, RuntimeError, AssertionError,
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

#!/usr/bin/env python3
"""One Linux SleepAPI benchmark case; run beside a fresh server in its PID/network namespaces.

No Docker API access is needed. PID 1 must be the owned SleepAPI process and
127.0.0.1:3000 its HTTP listener. Outputs are created in a new directory.
"""

import argparse
from concurrent.futures import ThreadPoolExecutor
from contextlib import contextmanager
from datetime import datetime, timezone
import http.client
import importlib.util
import json
import math
import os
from pathlib import Path
import platform
import re
import sys
import threading
import time


METHODS = {"tokio": "tokio_sleep", "tokio-spawn": "tokio_spawn",
           "thread": "thread_sleep", "park": "thread_park"}
MAX_BODY = 65536
MAX_REQUEST_RECORDS = 100_000
TIMEOUT = 2.0
SAMPLE_INTERVAL = 0.1
SPEC = importlib.util.spec_from_file_location(
    "benchmark_resources", Path(__file__).with_name("benchmark-resources.py"))
SAMPLER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(SAMPLER)


def require(condition, message):
    if not condition:
        raise AssertionError(message)


def percentile(values, fraction):
    require(bool(values), "cannot calculate a percentile without observations")
    ordered = sorted(values)
    return ordered[max(0, math.ceil(len(ordered) * fraction) - 1)]


def resource_summary(samples, start_ns, end_ns, ticks_per_second):
    inside = [sample for sample in samples if start_ns <= sample["monotonic_ns"] <= end_ns]
    require(len(inside) >= 2, "fewer than two resource samples inside measurement")
    first, last = inside[0], inside[-1]
    span = (last["monotonic_ns"] - first["monotonic_ns"]) / 1e9
    delta = (last["user_cpu_ticks"] + last["system_cpu_ticks"]
             - first["user_cpu_ticks"] - first["system_cpu_ticks"])
    require(span > 0 and delta >= 0, "invalid CPU counter span")
    return {"sample_count": len(inside), "cpu_counter_span_seconds": span,
            "cpu_ticks_delta": delta, "cpu_seconds": delta / ticks_per_second,
            "cpu_percent_one_core": 100 * delta / ticks_per_second / span,
            "rss_peak_bytes": max(s["rss_bytes"] for s in inside),
            "rss_first_bytes": first["rss_bytes"], "rss_last_bytes": last["rss_bytes"],
            "threads_peak": max(s["live_threads"] for s in inside),
            "threads_first": first["live_threads"], "threads_last": last["live_threads"],
            "first_sample_ns": first["monotonic_ns"], "last_sample_ns": last["monotonic_ns"],
            "leading_unsampled_ms": (first["monotonic_ns"] - start_ns) / 1e6,
            "trailing_unsampled_ms": (end_ns - last["monotonic_ns"]) / 1e6,
            "max_sample_gap_ms": max((b["monotonic_ns"] - a["monotonic_ns"]) / 1e6
                                     for a, b in zip(inside, inside[1:]))}


class Evidence:
    def __init__(self, directory):
        self.directory = directory
        directory.mkdir(parents=True, exist_ok=False)
        self.files = {name: (directory / (name + ".jsonl")).open(
            "x", encoding="utf-8", buffering=65536) for name in ("requests", "samples", "phases")}
        self.locks = {name: threading.Lock() for name in self.files}
        self.requests = []
        self.samples = []
        self.phases = []
        self.sample_error = None
        self.stop = threading.Event()
        self.thread = None
        self.ticks = os.sysconf("SC_CLK_TCK")
        self.page_size = os.sysconf("SC_PAGE_SIZE")
        self.started_ticks = None
        self.command = None

    def emit(self, category, record):
        # Separate streams avoid serializing resource samples behind request
        # logging. Buffer writes so the host bind mount is not flushed per response.
        encoded = json.dumps(record, allow_nan=False) + "\n"
        with self.locks[category]:
            if category == "requests":
                require(len(self.requests) < MAX_REQUEST_RECORDS, "request evidence limit reached")
            self.files[category].write(encoded)
            getattr(self, category).append(record)
            if category == "phases":
                self.files[category].flush()

    def phase(self, name, event, **extra):
        record = {"phase": name, "event": event, "monotonic_ns": time.monotonic_ns(), **extra}
        self.emit("phases", record)
        return record["monotonic_ns"]

    def start_sampler(self):
        require(sys.platform.startswith("linux"), "worker needs Linux /proc")
        require(os.getpid() != 1, "worker must not replace the target server as PID 1")
        self.command = Path("/proc/1/comm").read_text().strip()
        require(self.command == "sleepapi", "PID 1 is not SleepAPI")
        require(os.readlink("/proc/1/ns/pid") == os.readlink("/proc/self/ns/pid"),
                "worker must share SleepAPI's PID namespace")
        first = SAMPLER.process_sample(1, self.ticks, self.page_size)
        self.started_ticks = first["process_start_ticks"]
        self.emit("samples", {"server": first,
                              "client": SAMPLER.process_sample(os.getpid(), self.ticks, self.page_size)})

        def sample_loop():
            next_at = time.monotonic() + SAMPLE_INTERVAL
            try:
                while not self.stop.wait(max(0, next_at - time.monotonic())):
                    require(Path("/proc/1/comm").read_text().strip() == self.command,
                            "target process command changed")
                    record = {"server": SAMPLER.process_sample(1, self.ticks, self.page_size,
                                                               self.started_ticks),
                              "client": SAMPLER.process_sample(os.getpid(), self.ticks, self.page_size)}
                    self.emit("samples", record)
                    next_at = max(next_at + SAMPLE_INTERVAL, time.monotonic())
            except BaseException as error:
                self.sample_error = f"{type(error).__name__}: {error}"
                self.stop.set()
        self.thread = threading.Thread(target=sample_loop, daemon=True)
        self.thread.start()

    def close(self):
        self.stop.set()
        if self.thread:
            self.thread.join(timeout=3)
            require(not self.thread.is_alive(), "sampler did not stop")
        for handle in self.files.values():
            handle.close()

    def resources(self, start_ns, end_ns):
        require(self.sample_error is None, "resource sampler failed: " + str(self.sample_error))
        return {role: resource_summary([s[role] for s in self.samples], start_ns, end_ns, self.ticks)
                for role in ("server", "client")}


def connection(timeout=TIMEOUT):
    return http.client.HTTPConnection("127.0.0.1", 3000, timeout=timeout)


def observe(conn, method, duration_ms, phase, worker=0, sequence=0, path=None, sent=None):
    route = path or "/sleep/" + method
    record = {"phase": phase, "worker": worker, "sequence": sequence, "route": route,
              "duration_ms": duration_ms, "start_ns": time.monotonic_ns(),
              "status": None, "body": None, "error": None}
    try:
        conn.request("GET" if route == "/health" else "POST", route,
                     None if route == "/health" else json.dumps({"duration_ms": duration_ms}),
                     {} if route == "/health" else {"Content-Type": "application/json"})
        if sent is not None:
            sent.set()
        response = conn.getresponse()
        lengths = response.headers.get_all("Content-Length", [])
        require(response.getheader("Transfer-Encoding") is None,
                "expected Content-Length framing without Transfer-Encoding")
        require(len(lengths) == 1 and re.fullmatch(r"[0-9]+", lengths[0].strip()) is not None,
                "expected exactly one valid Content-Length")
        length = int(lengths[0])
        require(length <= MAX_BODY, "response body exceeds 64 KiB")
        require(response.getheader("Content-Type", "").split(";", 1)[0].strip().lower() == "application/json",
                "expected application/json response")
        # read() verifies complete framing; the validated length bounds its allocation.
        raw = response.read()
        require(len(raw) == length, "response did not match Content-Length")
        record.update(status=response.status, body=json.loads(raw), headers={
            name.lower(): value for name, value in response.getheaders()
            if name.lower() in ("content-type", "retry-after", "content-length")})
    except (OSError, http.client.HTTPException, ValueError, AssertionError) as error:
        record["error"] = {"type": type(error).__name__, "message": str(error)}
    finally:
        record["end_ns"] = time.monotonic_ns()
        record["latency_ms"] = (record["end_ns"] - record["start_ns"]) / 1e6
        if sent is not None:
            sent.set()
    return record


def validate_success(record, method, duration_ms):
    require(record["error"] is None, "request failed: " + str(record["error"]))
    body = record["body"]
    require(record["status"] == 200 and isinstance(body, dict), "expected HTTP 200 JSON object")
    require(body.get("simulated") is True and body.get("operation") == "sleep",
            "expected simulated sleep")
    require(body.get("status_code") == 200 and body.get("method") == METHODS[method],
            "wrong response status or waiting method")
    require(body.get("blocks_thread") is (method in ("thread", "park")), "wrong blocks_thread")
    require(body.get("requested_duration_ms") == duration_ms, "wrong requested duration")
    elapsed = body.get("elapsed_ms")
    require(type(elapsed) in (float, int) and math.isfinite(elapsed) and elapsed >= duration_ms,
            "invalid or early server elapsed time")
    require(record["latency_ms"] >= duration_ms, "client observed an early response")
    require("error" not in body, "successful simulation has error data")


def validate_capacity(record):
    require(record["error"] is None and record["status"] == 429, "expected HTTP 429")
    body = record["body"]
    require(isinstance(body, dict) and body.get("simulated") is False
            and body.get("status_code") == 429
            and body.get("error", {}).get("code") == "capacity_exceeded",
            "expected actual capacity rejection, not simulated rate limiting")
    require(record.get("headers", {}).get("retry-after") == "1", "missing capacity Retry-After")


def checked_request(evidence, method, duration_ms, phase, timeout=TIMEOUT, path=None):
    conn = connection(timeout)
    try:
        record = observe(conn, method, duration_ms, phase, path=path)
    finally:
        conn.close()
    evidence.emit("requests", record)
    return record


def load_phase(evidence, method, duration_ms, concurrency, seconds, phase):
    ready = threading.Barrier(concurrency + 1)
    go = threading.Event()
    abort = threading.Event()
    deadline = [None]

    def worker(index):
        conn = connection()
        records = []
        try:
            ready.wait(timeout=10)
            go.wait(timeout=10)
            sequence = 0
            while time.monotonic_ns() < deadline[0] and not abort.is_set():
                sequence += 1
                record = observe(conn, method, duration_ms, phase, index, sequence)
                try:
                    validate_success(record, method, duration_ms)
                    record["valid"] = True
                except AssertionError as error:
                    record["valid"] = False
                    record["validation_error"] = str(error)
                    abort.set()
                evidence.emit("requests", record)
                records.append(record)
        finally:
            conn.close()
        return records

    with ThreadPoolExecutor(max_workers=concurrency) as pool:
        futures = [pool.submit(worker, index) for index in range(concurrency)]
        ready.wait(timeout=10)
        started = evidence.phase(phase, "start", concurrency=concurrency)
        deadline[0] = started + round(seconds * 1e9)
        go.set()
        rows = [record for future in futures for record in future.result(timeout=seconds + 15)]
    ended = evidence.phase(phase, "drained", scheduled_end_ns=deadline[0])
    require(rows and all(row["valid"] for row in rows), "load produced unexpected response(s)")
    require(all(started <= row["start_ns"] < deadline[0] for row in rows),
            "request started outside the shared launch window")
    last_response = max(row["end_ns"] for row in rows)
    latencies = [row["latency_ms"] for row in rows]
    completed_in_window = sum(row["end_ns"] <= deadline[0] for row in rows)
    return {"start_ns": started, "scheduled_end_ns": deadline[0], "drain_end_ns": ended,
            "last_response_ns": last_response, "launch_window_seconds": seconds,
            "response_span_seconds": (last_response - started) / 1e9,
            "requests": len(rows), "successful_responses": len(rows), "unexpected_responses": 0,
            "completions_in_window": completed_in_window,
            "completed_in_window_per_second": completed_in_window / seconds,
            "drained_responses_per_second": len(rows) / ((last_response - started) / 1e9),
            "latency_ms": {"p50": percentile(latencies, .5), "p95": percentile(latencies, .95),
                           "max": max(latencies)},
            "resources": evidence.resources(started, last_response)}


def load(evidence, args):
    idle_start = evidence.phase("idle", "start")
    time.sleep(1)
    idle_end = evidence.phase("idle", "end")
    idle = evidence.resources(idle_start, idle_end)
    warmup = load_phase(evidence, args.method, args.duration_ms, args.concurrency, 2, "warmup")
    measured = load_phase(evidence, args.method, args.duration_ms, args.concurrency,
                          args.window_secs, "measured")
    return {"idle": idle, "warmup": warmup, "measured": measured,
            "delay_only_ceiling_responses_per_second": args.concurrency * 1000 / args.duration_ms,
            "connection_policy": "one persistent HTTP/1.1 connection per worker; fresh clients after server warmup"}


@contextmanager
def occupancy(evidence, args, timeout_seconds, phase, duration_ms=1500):
    """Start one long request and establish actual admission; zero-delay probes can win first."""
    for launch in range(1, 4):
        conn = connection(timeout_seconds)
        sent = threading.Event()
        with ThreadPoolExecutor(max_workers=1) as pool:
            future = pool.submit(observe, conn, args.method, duration_ms, phase + "_long", 0, launch, None, sent)
            require(sent.wait(3), "long request did not start")
            deadline = time.monotonic() + .2
            occupied = None
            while time.monotonic() < deadline and not future.done():
                probe = checked_request(evidence, args.method, 0, phase + "_admission")
                if probe["status"] == 429:
                    validate_capacity(probe)
                    occupied = probe
                    break
                validate_success(probe, args.method, 0)
                time.sleep(.005)
            if occupied is not None:
                try:
                    yield future, occupied
                finally:
                    conn.close()
                    result = future.result(timeout=3)
                    evidence.emit("requests", result)
                return
            result = future.result(timeout=3)
            evidence.emit("requests", result)
            conn.close()
            if result["status"] == 429:
                validate_capacity(result)
            else:
                raise AssertionError("could not establish occupied slot before long request outcome")
    raise AssertionError("long request lost admission three times")


def recovery(evidence, args, phase):
    deadline = time.monotonic() + 4
    last_rejection = None
    while time.monotonic() < deadline:
        row = checked_request(evidence, args.method, 0, phase)
        if row["status"] == 200:
            validate_success(row, args.method, 0)
            return row, last_rejection
        validate_capacity(row)
        last_rejection = row
        time.sleep(.01)
    raise AssertionError("simulation admission did not recover")


def lifecycle(evidence, args):
    started = evidence.phase("lifecycle", "start")
    overload = None
    with occupancy(evidence, args, TIMEOUT, "overload") as (future, occupied):
        with ThreadPoolExecutor(max_workers=16) as pool:
            rows = list(pool.map(lambda _: checked_request(evidence, args.method, 100, "overload_burst"), range(16)))
        for row in rows:
            validate_capacity(row)
        health = checked_request(evidence, args.method, 0, "overload_health", path="/health")
        require(health["error"] is None and health["status"] == 200
                and health["body"] == {"status": "ok"}, "health failed while occupied")
        long = future.result(timeout=3)
        validate_success(long, args.method, 1500)
        overload = {"admission_proven_ns": occupied["end_ns"], "rejections": len(rows),
                    "health_latency_ms": health["latency_ms"], "long_request_latency_ms": long["latency_ms"]}
    admitted, _ = recovery(evidence, args, "overload_recovery")
    overload["recovered_ns"] = admitted["end_ns"]
    cancellation = None
    with occupancy(evidence, args, .3, "cancellation") as (future, occupied):
        timed_out = future.result(timeout=2)
        require(timed_out["error"] is not None and timed_out["error"]["type"] in ("TimeoutError", "timeout"),
                "long request must end at the client's timeout")
        # Exiting occupancy closes the timed-out socket before recovery probes.
        cancellation = {"request_start_ns": timed_out["start_ns"], "timeout_observed_ns": timed_out["end_ns"],
                        "admission_proven_ns": occupied["end_ns"], "requested_duration_ms": 1500,
                        "client_timeout_ms": 300}
    admitted, rejected = recovery(evidence, args, "cancellation_recovery")
    cancellation.update(first_success_start_ns=admitted["start_ns"], first_success_end_ns=admitted["end_ns"],
                        last_rejection_start_ns=rejected["start_ns"] if rejected else None,
                        last_rejection_end_ns=rejected["end_ns"] if rejected else None,
                        recovery_from_request_start_ms=(admitted["end_ns"]-cancellation["request_start_ns"])/1e6,
                        recovery_after_timeout_ms=(admitted["end_ns"]-cancellation["timeout_observed_ns"])/1e6)
    ended = evidence.phase("lifecycle", "end")
    return {"overload": overload, "cancellation": cancellation,
            "resources": evidence.resources(started, ended)}


def bounded_float(low, high):
    def parse(value):
        value = float(value)
        if not math.isfinite(value) or not low <= value <= high:
            raise argparse.ArgumentTypeError(f"must be between {low} and {high}")
        return value
    return parse


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--mode", choices=("load", "lifecycle"), required=True)
    parser.add_argument("--method", choices=METHODS, required=True)
    parser.add_argument("--output-dir", type=Path, required=True)
    parser.add_argument("--window-secs", type=bounded_float(1, 60), default=10)
    parser.add_argument("--concurrency", type=int, choices=range(1, 129), default=1)
    parser.add_argument("--duration-ms", type=int, choices=range(1, 1001), default=100)
    args = parser.parse_args()
    report = {"schema_version": 1, "passed": False, "mode": args.mode, "method": args.method,
              "observed_at_utc": datetime.now(timezone.utc).isoformat(), "platform": platform.platform(),
              "python_version": platform.python_version(), "configuration": vars(args).copy(),
              "cpu_convention": "100% = one logical CPU; process CPU only",
              "rss_convention": "process resident pages; sampled peaks exclude other cgroup memory",
              "sample_interval_ms": 100, "request_timeout_seconds": TIMEOUT,
              "max_response_body_bytes": MAX_BODY, "max_request_records": MAX_REQUEST_RECORDS,
              "evidence_io": "64-KiB buffered JSONL streams; phase records flushed immediately, all streams on close"}
    report["configuration"]["output_dir"] = str(args.output_dir)
    evidence = None
    try:
        evidence = Evidence(args.output_dir)
        evidence.start_sampler()
        report.update(target_pid=1, process_start_ticks=evidence.started_ticks,
                      clock_ticks_per_second=evidence.ticks, page_size_bytes=evidence.page_size,
                      pid_namespace=os.readlink("/proc/1/ns/pid"),
                      target_cgroup=Path("/proc/1/cgroup").read_text().splitlines(),
                      target_comm=Path("/proc/1/comm").read_text().strip(),
                      target_cmdline=Path("/proc/1/cmdline").read_bytes().decode().split("\0"),
                      target_allowed_cpus=[line.split(":", 1)[1].strip()
                                           for line in Path("/proc/1/status").read_text().splitlines()
                                           if line.startswith("Cpus_allowed_list:")][0],
                      client_pid=os.getpid(), client_allowed_cpus=sorted(os.sched_getaffinity(0)),
                      sampler_identity="shared PID namespace PID 1 comm/start-ticks; no Docker host identity claim")
        health = checked_request(evidence, args.method, 0, "readiness", path="/health")
        require(health["error"] is None and health["status"] == 200 and health["body"] == {"status": "ok"},
                "server is not healthy")
        report["results"] = load(evidence, args) if args.mode == "load" else lifecycle(evidence, args)
        require(evidence.sample_error is None, str(evidence.sample_error))
        report["passed"] = True
    except (OSError, ValueError, AssertionError, RuntimeError, TimeoutError) as error:
        report["error"] = f"{type(error).__name__}: {error}"
    finally:
        if evidence is not None:
            try:
                evidence.close()
            except (OSError, AssertionError) as error:
                report["passed"] = False
                report["cleanup_error"] = f"{type(error).__name__}: {error}"
            if evidence.sample_error:
                report["passed"] = False
                report["sample_error"] = evidence.sample_error
            report["request_records"] = len(evidence.requests)
            report["resource_samples"] = len(evidence.samples)
            (args.output_dir / "summary.json").write_text(json.dumps(report, indent=2, allow_nan=False) + "\n")
    print(json.dumps({"passed": report["passed"], "mode": args.mode, "method": args.method,
                      "output_dir": str(args.output_dir), "error": report.get("error")}))
    return 0 if report["passed"] else 1


if __name__ == "__main__":
    sys.exit(main())

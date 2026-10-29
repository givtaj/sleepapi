#!/usr/bin/env python3
"""Sample one local Docker container's init process without sending HTTP traffic.

Usage:
    python3 scripts/benchmark-resources.py --container sleepapi-benchmark \
        --output /tmp/sleepapi-resources.jsonl --max-duration-secs 120 \
        --stop-file /tmp/sleepapi-resources.stop

Output is created exclusively (existing files are never overwritten). Create the
stop file, or send SIGINT/SIGTERM, to finish early and flush the final record.
The Docker daemon and this script must share the host's PID/cgroup view. Samples
cover the container init process, including its live OS threads, not child
processes. This is appropriate for SleepAPI's single-process Docker image.
CPU percent uses process CPU time / elapsed wall time: 100% is one logical CPU,
so multithreaded work can exceed 100%. RSS is resident process memory, not the
container's cgroup memory usage. Peaks between samples can be missed.
"""

import argparse
from datetime import datetime, timezone
import json
import math
import os
from pathlib import Path
import re
import signal
import subprocess
import sys
import time


PROC_ROOT = Path("/proc")


def bounded_number(low, high):
    def parse(value):
        try:
            number = float(value)
        except ValueError as error:
            raise argparse.ArgumentTypeError("must be a number") from error
        if not math.isfinite(number) or not low <= number <= high:
            raise argparse.ArgumentTypeError(f"must be between {low} and {high}")
        return number
    return parse


def utc_now():
    return datetime.now(timezone.utc).isoformat()


def inspect_container(reference):
    result = subprocess.run(
        ["docker", "inspect", "--type", "container", "--format", "{{json .}}", "--", reference],
        capture_output=True, text=True, timeout=15, check=True,
    )
    details = json.loads(result.stdout)
    container_id = details.get("Id", "")
    state = details.get("State", {})
    pid = state.get("Pid")
    if not re.fullmatch(r"[0-9a-f]{64}", container_id):
        raise ValueError("Docker did not resolve a full container ID")
    if state.get("Running") is not True or type(pid) is not int or pid <= 0:
        raise ValueError("the selected container must be running with a valid host PID")
    cgroup = (PROC_ROOT / str(pid) / "cgroup").read_text()
    if container_id not in cgroup:
        raise ValueError(
            "container ID is absent from the local PID's cgroup; cannot verify the process. "
            "Use the Docker daemon's Linux host with a visible host /proc/cgroup view"
        )
    return details, cgroup


def process_sample(pid, clock_ticks, page_size, expected_start_ticks=None):
    # The parenthesized command can contain spaces and ')'; the final ')' closes it.
    stat = (PROC_ROOT / str(pid) / "stat").read_text()
    prefix, separator, suffix = stat.rpartition(") ")
    if not separator or int(prefix.split("(", 1)[0].strip()) != pid:
        raise ValueError("malformed process stat or unexpected PID")
    fields = suffix.split()  # Starts at stat field 3 (process state).
    if len(fields) < 22:
        raise ValueError("process stat is missing required fields")
    started = int(fields[19])  # stat field 22
    if expected_start_ticks is not None and started != expected_start_ticks:
        raise RuntimeError("PID was reused; refusing to sample a different process")
    if fields[0] in ("Z", "X", "x"):
        raise RuntimeError("container init process has exited")
    user_ticks, system_ticks = int(fields[11]), int(fields[12])
    rss_pages = int(fields[21])
    return {
        "monotonic_ns": time.monotonic_ns(),
        "process_start_ticks": started,
        "user_cpu_ticks": user_ticks,
        "system_cpu_ticks": system_ticks,
        "cpu_seconds": (user_ticks + system_ticks) / clock_ticks,
        "rss_bytes": max(0, rss_pages) * page_size,
        "live_threads": int(fields[17]),
    }


def run(args, output):
    requested_signal = None

    def handle_signal(signum, _frame):
        nonlocal requested_signal
        # A signal can interrupt a wait while its internal lock is held. Keep
        # the handler lock-free; Event.set() here can deadlock Event.wait().
        requested_signal = signum

    original_handlers = {
        signum: signal.signal(signum, handle_signal)
        for signum in (signal.SIGINT, signal.SIGTERM)
    }

    def emit(record):
        output.write(json.dumps(record, allow_nan=False, sort_keys=True) + "\n")
        output.flush()

    count = 0
    started_ns = time.monotonic_ns()
    reason = "error"
    exit_code = 0
    try:
        details, cgroup = inspect_container(args.container)
        pid = details["State"]["Pid"]
        clock_ticks = os.sysconf("SC_CLK_TCK")
        page_size = os.sysconf("SC_PAGE_SIZE")
        first = process_sample(pid, clock_ticks, page_size)
        # Recheck the process identity after reading cgroup during inspection.
        verified_cgroup = (PROC_ROOT / str(pid) / "cgroup").read_text()
        if details["Id"] not in verified_cgroup:
            raise RuntimeError("container process identity changed during inspection")
        started_ns = first["monotonic_ns"]
        host_config = details.get("HostConfig", {})
        emit({
            "type": "metadata", "schema_version": 1, "started_at_utc": utc_now(),
            "container_reference": args.container, "container_id": details["Id"],
            "container_name": details.get("Name", "").lstrip("/"),
            "image_id": details.get("Image"), "pid": pid,
            "process_start_ticks": first["process_start_ticks"], "cgroup": cgroup.splitlines(),
            "clock_ticks_per_second": clock_ticks, "page_size_bytes": page_size,
            "host_logical_cpus": os.cpu_count(), "scope": "container_init_process",
            "cpu_percent_convention": "100% = one logical CPU; may exceed 100%",
            "rss_convention": "resident process pages; not container cgroup memory",
            "configured_container_limits": {
                key: host_config.get(key)
                for key in ("NanoCpus", "CpuPeriod", "CpuQuota", "CpusetCpus", "Memory")
            },
            "interval_ms": args.interval_ms, "max_duration_secs": args.max_duration_secs,
            "stop_file": str(args.stop_file) if args.stop_file else None,
        })
        previous = None
        sample = first
        interval_ns = int(args.interval_ms * 1_000_000)
        deadline_ns = started_ns + int(args.max_duration_secs * 1_000_000_000)
        next_sample_ns = started_ns
        while True:
            delta_ns = sample["monotonic_ns"] - previous["monotonic_ns"] if previous else None
            delta_ticks = (
                sample["user_cpu_ticks"] + sample["system_cpu_ticks"]
                - previous["user_cpu_ticks"] - previous["system_cpu_ticks"]
                if previous else None
            )
            if previous and (delta_ns <= 0 or delta_ticks < 0):
                raise RuntimeError("process CPU counters or sample clock moved backwards")
            emit({
                "type": "sample", "sample_index": count,
                "elapsed_seconds": (sample["monotonic_ns"] - started_ns) / 1_000_000_000,
                "cpu_percent": (
                    100 * delta_ticks / clock_ticks / (delta_ns / 1_000_000_000)
                    if previous else None
                ),
                **sample,
            })
            count += 1
            previous = sample
            if requested_signal is not None:
                reason = signal.Signals(requested_signal).name
                break
            if args.stop_file and args.stop_file.exists():
                reason = "stop_file"
                break
            now_ns = time.monotonic_ns()
            if now_ns >= deadline_ns:
                reason = "max_duration"
                break
            # Skip missed slots instead of emitting a burst of delayed samples.
            next_sample_ns = max(next_sample_ns + interval_ns, now_ns)
            wake_ns = min(next_sample_ns, deadline_ns)
            # Poll stop requests at most 100 ms apart even with a long sample
            # interval. Polling must not create additional resource samples.
            while requested_signal is None:
                if args.stop_file and args.stop_file.exists():
                    break
                remaining_secs = (wake_ns - time.monotonic_ns()) / 1_000_000_000
                if remaining_secs <= 0:
                    break
                time.sleep(min(0.1, remaining_secs))
            if requested_signal is not None:
                reason = signal.Signals(requested_signal).name
                break
            if args.stop_file and args.stop_file.exists():
                reason = "stop_file"
                break
            sample = process_sample(pid, clock_ticks, page_size, first["process_start_ticks"])
    except (OSError, ValueError, RuntimeError, subprocess.SubprocessError) as error:
        # Do not emit full Docker inspect data (it can contain environment secrets).
        message = str(error)
        emit({"type": "error", "error": type(error).__name__, "message": message})
        print(f"Resource sampling failed: {message}", file=sys.stderr)
        exit_code = 1
    finally:
        try:
            emit({
                "type": "summary", "finished_at_utc": utc_now(), "reason": reason,
                "sample_count": count, "elapsed_seconds": (time.monotonic_ns() - started_ns) / 1_000_000_000,
            })
        finally:
            for signum, handler in original_handlers.items():
                signal.signal(signum, handler)
    return exit_code


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--container", required=True, help="explicit name or ID of an already-running local container")
    parser.add_argument("--output", required=True, type=Path, help="new JSONL output file; parent directory must exist")
    parser.add_argument("--interval-ms", type=bounded_number(10, 10000), default=100, help="sample interval, 10–10000 ms (default: 100)")
    parser.add_argument("--max-duration-secs", type=bounded_number(0.1, 900), default=900, help="maximum sample duration, 0.1–900 seconds (default: 900)")
    parser.add_argument("--stop-file", type=Path, help="finish when this path exists; never created or removed by the sampler")
    args = parser.parse_args()
    if not sys.platform.startswith("linux"):
        parser.error("sampling requires Linux /proc on the Docker daemon's host")
    try:
        with args.output.open("x", encoding="utf-8") as output:
            return run(args, output)
    except OSError as error:
        print(f"Cannot create or write output: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())

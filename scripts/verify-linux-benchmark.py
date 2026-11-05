#!/usr/bin/env python3
"""Independently audit a completed benchmark-linux.py evidence directory.

Reads retained HTTP records and process counters without importing the runner.
Accepts either JSONL or gzip-compressed JSONL. Writes a new audit receipt and
SHA-256 inventory; it never changes measurement inputs or sends HTTP traffic.
"""

import argparse
from collections import Counter, defaultdict
from datetime import datetime, timezone
import gzip
import hashlib
import json
import math
from pathlib import Path
import re
import tarfile


METHODS = {"tokio": "tokio_sleep", "tokio-spawn": "tokio_spawn",
           "thread": "thread_sleep", "park": "thread_park"}


def require(value, message):
    if not value:
        raise AssertionError(message)


def read_json(path):
    with path.open() as source:
        return json.load(source)


def rows(path):
    compressed = path.with_suffix(path.suffix + ".gz")
    require(path.exists() != compressed.exists(), f"need exactly one raw/compressed file: {path}")
    opener, selected = (path.open, path) if path.exists() else (gzip.open, compressed)
    if path.exists():
        with opener("r") as source:
            return [json.loads(line) for line in source]
    with opener(selected, "rt") as source:
        return [json.loads(line) for line in source]


def close(actual, expected, label):
    if isinstance(expected, bool):
        require(actual is expected, f"{label}: {actual!r} is not {expected!r}")
    elif isinstance(expected, float):
        require(type(actual) in (float, int) and math.isfinite(actual)
                and math.isclose(actual, expected, rel_tol=1e-9, abs_tol=1e-6),
                f"{label}: {actual!r} != {expected!r}")
    else:
        require(actual == expected, f"{label}: {actual!r} != {expected!r}")


def compare_fields(actual, calculated, label):
    for key, value in calculated.items():
        require(key in actual, f"{label}: missing {key}")
        if isinstance(value, dict):
            compare_fields(actual[key], value, label + "." + key)
        else:
            close(actual[key], value, label + "." + key)


def cpus(value):
    if isinstance(value, list):
        return set(value)
    result = set()
    for span in value.split(","):
        ends = span.split("-")
        result.update(range(int(ends[0]), int(ends[-1]) + 1))
    return result


def one(records, field, value):
    selected = [record for record in records if record.get(field) == value]
    require(len(selected) == 1, f"expected one {field}={value}, got {len(selected)}")
    return selected[0]


def phase(phases, name, event):
    matches = [p for p in phases if p["phase"] == name and p["event"] == event]
    require(len(matches) == 1, f"expected one phase {name}/{event}")
    return matches[0]


def success(row, method, duration):
    require(row["error"] is None and row["status"] == 200, "unexpected success response")
    body = row["body"]
    require(isinstance(body, dict), "success body must be an object")
    expected = {"simulated": True, "operation": "sleep", "status_code": 200,
                "method": METHODS[method], "requested_duration_ms": duration,
                "blocks_thread": method in ("thread", "park")}
    for key, value in expected.items():
        close(body.get(key), value, "body." + key)
    elapsed = body.get("elapsed_ms")
    require(type(elapsed) in (int, float) and math.isfinite(elapsed) and elapsed >= duration,
            "early or invalid server elapsed time")
    require(row["latency_ms"] >= duration and "error" not in body, "invalid success timing/error")


def capacity(row):
    body = row["body"]
    require(row["error"] is None and row["status"] == 429 and isinstance(body, dict),
            "expected complete capacity rejection")
    require(body.get("simulated") is False and body.get("status_code") == 429
            and body.get("error", {}).get("code") == "capacity_exceeded", "wrong 429 contract")
    require(row["headers"].get("retry-after") == "1", "wrong Retry-After")


def resources(raw, begin, end, hz):
    inside = [row for row in raw if begin <= row["monotonic_ns"] <= end]
    require(len(inside) >= 2, "insufficient samples in phase")
    first, last = inside[0], inside[-1]
    seconds = (last["monotonic_ns"] - first["monotonic_ns"]) / 1e9
    ticks = sum(last[k] - first[k] for k in ("user_cpu_ticks", "system_cpu_ticks"))
    require(seconds > 0 and ticks >= 0, "invalid CPU span")
    return {"sample_count": len(inside), "cpu_counter_span_seconds": seconds,
            "cpu_ticks_delta": ticks, "cpu_seconds": ticks / hz,
            "cpu_percent_one_core": ticks / hz / seconds * 100,
            "rss_peak_bytes": max(s["rss_bytes"] for s in inside),
            "rss_first_bytes": first["rss_bytes"], "rss_last_bytes": last["rss_bytes"],
            "threads_peak": max(s["live_threads"] for s in inside),
            "threads_first": first["live_threads"], "threads_last": last["live_threads"],
            "first_sample_ns": first["monotonic_ns"], "last_sample_ns": last["monotonic_ns"],
            "leading_unsampled_ms": (first["monotonic_ns"] - begin) / 1e6,
            "trailing_unsampled_ms": (end - last["monotonic_ns"]) / 1e6,
            "max_sample_gap_ms": max((b["monotonic_ns"] - a["monotonic_ns"]) / 1e6
                                     for a, b in zip(inside, inside[1:]))}


def audit_load(summary, requests, samples, phases, cell):
    result = summary["results"]
    hz = summary["clock_ticks_per_second"]
    require(Counter((p["phase"], p["event"]) for p in phases) == Counter([
        ("idle", "start"), ("idle", "end"), ("warmup", "start"),
        ("warmup", "drained"), ("measured", "start"), ("measured", "drained")]),
        "unexpected load phases")
    require({r["phase"] for r in requests} == {"readiness", "warmup", "measured"},
            "unexpected load request phase")
    idle_start = phase(phases, "idle", "start")["monotonic_ns"]
    idle_end = phase(phases, "idle", "end")["monotonic_ns"]
    require(idle_end - idle_start >= 1e9, "idle phase shorter than one second")
    require(one(requests, "phase", "readiness")["end_ns"] <= idle_start, "readiness overlaps idle")
    for role in ("server", "client"):
        compare_fields(result["idle"][role], resources([s[role] for s in samples], idle_start, idle_end, hz),
                       "idle." + role)
    prior_end = idle_end
    calculated = {}
    for name in ("warmup", "measured"):
        records = [r for r in requests if r["phase"] == name]
        require(records, f"missing {name} records")
        start = phase(phases, name, "start")["monotonic_ns"]
        drain = phase(phases, name, "drained")
        deadline, end = drain["scheduled_end_ns"], drain["monotonic_ns"]
        close((deadline - start) / 1e9,
              2.0 if name == "warmup" else summary["configuration"]["window_secs"], name + " window")
        close(phase(phases, name, "start")["concurrency"], cell["concurrency"], name + " concurrency")
        require(start >= prior_end and deadline > start and end >= deadline, "phase overlap/bad deadline")
        require(all(start <= r["start_ns"] < deadline and r["end_ns"] <= end for r in records),
                "request outside its launch/drain window")
        workers = defaultdict(list)
        for record in records:
            require(record["route"] == "/sleep/" + cell["method"], "wrong measured route")
            success(record, cell["method"], summary["configuration"]["duration_ms"])
            require(record.get("valid") is True, "worker marked a request invalid")
            workers[record["worker"]].append(record)
        require(set(workers) == set(range(cell["concurrency"])), "missing or unexpected client worker")
        for records_by_worker in workers.values():
            ordered = sorted(records_by_worker, key=lambda r: r["sequence"])
            require([r["sequence"] for r in ordered] == list(range(1, len(ordered) + 1)), "sequence gap")
            require(all(a["end_ns"] <= b["start_ns"] for a, b in zip(ordered, ordered[1:])),
                    "closed-loop worker has overlapping requests")
        last = max(r["end_ns"] for r in records)
        seconds = (deadline - start) / 1e9
        span = (last - start) / 1e9
        latency = sorted(r["latency_ms"] for r in records)
        within = sum(r["end_ns"] <= deadline for r in records)
        metrics = {"start_ns": start, "scheduled_end_ns": deadline, "drain_end_ns": end,
                   "last_response_ns": last, "launch_window_seconds": seconds,
                   "response_span_seconds": span, "requests": len(records),
                   "successful_responses": len(records), "unexpected_responses": 0,
                   "completions_in_window": within, "completed_in_window_per_second": within / seconds,
                   "drained_responses_per_second": len(records) / span,
                   "latency_ms": {"p50": latency[math.ceil(.5 * len(latency)) - 1],
                                  "p95": latency[math.ceil(.95 * len(latency)) - 1], "max": latency[-1]},
                   "resources": {role: resources([s[role] for s in samples], start, last, hz)
                                 for role in ("server", "client")}}
        compare_fields(result[name], metrics, name)
        calculated[name] = metrics
        prior_end = end
    close(result["delay_only_ceiling_responses_per_second"],
          cell["concurrency"] * 1000 / summary["configuration"]["duration_ms"], "delay ceiling")
    return calculated["measured"]


def audit_lifecycle(summary, requests, samples, phases, cell):
    def selected(name):
        return [r for r in requests if r["phase"] == name]

    result = summary["results"]
    require(Counter((p["phase"], p["event"]) for p in phases) == Counter([
        ("lifecycle", "start"), ("lifecycle", "end")]), "unexpected lifecycle phases")
    require({r["phase"] for r in requests} <= {
        "readiness", "overload_long", "overload_admission", "overload_burst", "overload_health",
        "overload_recovery", "cancellation_long", "cancellation_admission", "cancellation_recovery"},
        "unexpected lifecycle request phase")
    begin = phase(phases, "lifecycle", "start")["monotonic_ns"]
    end = phase(phases, "lifecycle", "end")["monotonic_ns"]
    require(end > begin, "invalid lifecycle interval")
    require(one(requests, "phase", "readiness")["end_ns"] <= begin, "readiness overlaps lifecycle")
    require(all(begin <= r["start_ns"] < r["end_ns"] <= end
                for r in requests if r["phase"] != "readiness"), "request outside lifecycle phase")
    for role in ("server", "client"):
        compare_fields(result["resources"][role],
                       resources([s[role] for s in samples], begin, end, summary["clock_ticks_per_second"]), role)
    overload = result["overload"]
    admitted = one(selected("overload_long"), "status", 200)
    success(admitted, cell["method"], 1500)
    proof = one(selected("overload_admission"), "end_ns", overload["admission_proven_ns"])
    capacity(proof)
    require(admitted["start_ns"] < proof["start_ns"] <= proof["end_ns"] < admitted["end_ns"],
            "overload admission was not demonstrated during long request")
    burst = selected("overload_burst")
    require(len(burst) == 16 == overload["rejections"], "wrong overload burst size")
    for row in burst:
        capacity(row)
        require(proof["end_ns"] <= row["start_ns"] < row["end_ns"] < admitted["end_ns"],
                "overload burst outside occupied interval")
    health = one(requests, "phase", "overload_health")
    require(health["error"] is None and health["status"] == 200 and health["body"] == {"status": "ok"},
            "health failed during overload")
    require(proof["end_ns"] <= health["start_ns"] < health["end_ns"] < admitted["end_ns"],
            "health was not observed while occupied")
    close(overload["health_latency_ms"], health["latency_ms"], "health latency")
    close(overload["long_request_latency_ms"], admitted["latency_ms"], "long latency")
    overloaded_recovery = one(selected("overload_recovery"), "status", 200)
    success(overloaded_recovery, cell["method"], 0)
    require(overloaded_recovery["start_ns"] >= admitted["end_ns"], "overload recovery predates long response")
    close(overload["recovered_ns"], overloaded_recovery["end_ns"], "overload recovery")

    cancellation = result["cancellation"]
    require(cancellation["requested_duration_ms"] == 1500 and cancellation["client_timeout_ms"] == 300,
            "lifecycle cancellation configuration differs")
    timed_out = one(selected("cancellation_long"), "start_ns", cancellation["request_start_ns"])
    require(timed_out["status"] is None and timed_out["error"]["type"] in ("TimeoutError", "timeout"),
            "cancellation was not a client timeout")
    close(timed_out["end_ns"], cancellation["timeout_observed_ns"], "timeout timestamp")
    require(timed_out["latency_ms"] >= cancellation["client_timeout_ms"], "client timed out early")
    require(timed_out["start_ns"] >= overloaded_recovery["end_ns"], "cancellation overlaps overload recovery")
    proof = one(selected("cancellation_admission"), "end_ns", cancellation["admission_proven_ns"])
    capacity(proof)
    require(timed_out["start_ns"] <= proof["start_ns"] < proof["end_ns"] < timed_out["end_ns"],
            "cancellation admission was not proven before timeout")
    attempts = sorted(selected("cancellation_recovery"), key=lambda r: r["start_ns"])
    require(attempts and all(r["start_ns"] >= timed_out["end_ns"] for r in attempts),
            "recovery probe predates timeout")
    require(all(a["end_ns"] <= b["start_ns"] for a, b in zip(attempts, attempts[1:])),
            "sequential recovery probes overlap")
    for row in attempts[:-1]:
        capacity(row)
    last = attempts[-1]
    success(last, cell["method"], 0)
    previous = attempts[-2] if len(attempts) > 1 else None
    metrics = {"first_success_start_ns": last["start_ns"], "first_success_end_ns": last["end_ns"],
               "last_rejection_start_ns": previous["start_ns"] if previous else None,
               "last_rejection_end_ns": previous["end_ns"] if previous else None,
               "recovery_from_request_start_ms": (last["end_ns"] - timed_out["start_ns"]) / 1e6,
               "recovery_after_timeout_ms": (last["end_ns"] - timed_out["end_ns"]) / 1e6}
    compare_fields(cancellation, metrics, "cancellation")
    for row in requests:
        if row["phase"].endswith("_admission") or row["phase"] in ("overload_recovery", "cancellation_recovery"):
            capacity(row) if row["status"] == 429 else success(row, cell["method"], 0)
        elif row["phase"].endswith("_long") and row not in (admitted, timed_out):
            capacity(row)
    return {"overload_rejections": len(burst), **metrics}


def audit_provenance(root, environment):
    provenance = read_json(root / "provenance.json")
    require(provenance["source_commit"] == environment["source_commit"], "source provenance commit differs")
    require(provenance["source_export"] == ["git", "archive", environment["source_commit"]],
            "unexpected source export command")
    for filename, field in (("source.tar.gz", "source_archive_sha256"), ("build.log", "build_log_sha256")):
        require(hashlib.sha256((root / filename).read_bytes()).hexdigest() == provenance[field],
                f"provenance hash mismatch: {filename}")
    source_hashes = provenance["source_file_sha256"]
    require({"Cargo.toml", "Cargo.lock", "Dockerfile", ".dockerignore"} <= set(source_hashes),
            "source manifest misses a required build input")
    with tarfile.open(root / "source.tar.gz", "r:gz") as archive:
        members = archive.getmembers()
        require(len({m.name for m in members}) == len(members), "duplicate source archive member")
        if "comment" in archive.pax_headers:
            require(archive.pax_headers["comment"] == environment["source_commit"], "git archive commit differs")
        rust_sources = {m.name for m in members if m.isfile() and m.name.startswith("src/")
                        and m.name.endswith(".rs")}
        require(rust_sources and rust_sources <= set(source_hashes), "source manifest misses Rust source")
        for filename, expected_hash in source_hashes.items():
            member = archive.getmember(filename)
            require(member.isfile(), f"source hash is not a regular archive file: {filename}")
            # Read member bytes directly: never extract an archive to the filesystem.
            with archive.extractfile(member) as source:
                actual_hash = hashlib.sha256(source.read()).hexdigest()
            require(actual_hash == expected_hash, f"source member hash mismatch: {filename}")
    return {"source_files_verified": len(source_hashes),
            "source_archive_sha256": provenance["source_archive_sha256"],
            "build_log_sha256": provenance["build_log_sha256"]}


def audit_compression(root):
    compressed = {str(path.relative_to(root)) for path in root.rglob("*.jsonl.gz")}
    if not compressed:
        return {"compressed_streams_verified": 0}
    receipt = read_json(root / "compression.json")
    require(receipt["codec"] == "gzip" and receipt["lossless_roundtrip_verified"] is True,
            "missing lossless compression receipt")
    entries = receipt["records"]
    require(len({entry["path"] for entry in entries}) == len(entries)
            and {entry["path"] for entry in entries} == compressed, "compression manifest is incomplete/duplicated")
    for entry in entries:
        relative = Path(entry["path"])
        require(not relative.is_absolute() and ".." not in relative.parts, "invalid compression path")
        path = root / relative
        encoded = path.read_bytes()
        raw = gzip.decompress(encoded)
        require(len(encoded) == entry["compressed_bytes"] and len(raw) == entry["raw_bytes"]
                and hashlib.sha256(raw).hexdigest() == entry["raw_sha256"],
                "compressed bytes differ from retained raw digest: " + entry["path"])
    return {"compressed_streams_verified": len(entries)}


def audit(root):
    environment = read_json(root / "environment.json")
    plan = read_json(root / "plan.json")
    execution = read_json(root / "execution.json")
    require(environment["schema_version"] == 1, "unsupported environment schema")
    require(re.fullmatch(r"[0-9a-f]{40}", environment["source_commit"]), "invalid source commit")
    require(re.fullmatch(r"[0-9a-f]{64}", environment["binary_sha256"]), "invalid binary hash")
    require(environment["image_source_revision"] == environment["source_commit"], "image source label differs")
    require(environment["server_image"]["Os"] == environment["runner_image"]["Os"] == "linux",
            "server/runner is not Linux")
    require(environment["server_image"]["Architecture"] == environment["runner_image"]["Architecture"],
            "server/runner architectures differ")
    require(set(environment["harness_sha256"]) == {
        "benchmark-linux.py", "benchmark-worker.py", "benchmark-resources.py"}, "incomplete harness hashes")
    provenance_result = audit_provenance(root, environment)
    compression_result = audit_compression(root)
    for filename, expected in environment["harness_sha256"].items():
        path = root / "harness" / filename
        require(path.is_file(), f"missing frozen harness: {path}")
        require(hashlib.sha256(path.read_bytes()).hexdigest() == expected, f"harness hash mismatch: {filename}")
    repetitions = range(1, 2 if plan["pilot"] else 4)
    levels = [16] if plan["pilot"] else [1, 16, 64]
    expected = Counter((mode, method, concurrency, rep) for rep in repetitions for method in METHODS
                       for mode, concurrency in [("load", c) for c in levels] + [("lifecycle", 1)])
    actual = Counter((c["mode"], c["method"], c["concurrency"], c["repetition"]) for c in plan["cells"])
    require(expected == actual, "planned matrix is incomplete or duplicated")
    require(len({c["id"] for c in plan["cells"]}) == len(plan["cells"]), "case directory reused")
    require(execution["passed"] and execution["completed"] == execution["expected"] == len(plan["cells"]),
            "execution is incomplete")
    require(len(execution["results"]) == len(plan["cells"]), "execution result count differs")
    identities = set()
    audit_rows = []
    total_requests = total_samples = 0
    for cell, execution_result in zip(plan["cells"], execution["results"]):
        require(execution_result["case"] == cell, "execution order differs from fixed plan")
        directory = root / cell["id"]
        summary = read_json(directory / "data" / "summary.json")
        require(summary["schema_version"] == 1, "unsupported worker summary schema")
        require(summary == execution_result["summary"], f"summary copy differs: {cell['id']}")
        require(summary["passed"] and summary["mode"] == cell["mode"] and summary["method"] == cell["method"],
                f"wrong or failed case: {cell['id']}")
        config = summary["configuration"]
        require(config["concurrency"] == cell["concurrency"] and config["window_secs"] == plan["window_seconds"],
                "case configuration differs from plan")
        require(config["mode"] == cell["mode"] and config["method"] == cell["method"]
                and config["duration_ms"] == 100 and config["output_dir"] == "/out/data", "wrong workload configuration")
        require(summary["clock_ticks_per_second"] > 0 and summary["page_size_bytes"] > 0
                and summary["sample_interval_ms"] == 100, "invalid resource sampling configuration")
        server, after, worker = (read_json(directory / name) for name in
                                 ("server.json", "server-after.json", "worker.json"))
        require(server["id"] not in identities and worker["id"] not in identities, "container reused")
        require(server["id"] != worker["id"] and all(re.fullmatch(r"[0-9a-f]{64}", item["id"])
                                                      for item in (server, worker)), "invalid full container IDs")
        identities.update((server["id"], worker["id"]))
        require(server == after, "server identity changed during case")
        require(server["pid"] > 0 and server["image"] == environment["server_image"]["Id"], "wrong server image/PID")
        require(worker["image"] == environment["runner_image"]["Id"], "wrong runner image")
        require(worker["pid_mode"] == worker["network_mode"] == "container:" + server["id"],
                "worker does not join exact server namespaces")
        require(server["network_mode"] == "none", "server network unexpectedly exposed")
        for container in (server, worker):
            require(container["user"] == "65532:65532", "unexpected process user")
            compare_fields(container["limits"], {"Memory": 512 * 1024**2, "MemorySwap": 512 * 1024**2,
                                                 "PidsLimit": 256, "ReadonlyRootfs": True}, "container limits")
        server_environment = dict(value.split("=", 1) for value in server["environment"])
        compare_fields(server_environment, {"SLEEPAPI_ADDR": "127.0.0.1:3000", "TOKIO_WORKER_THREADS": "2",
                       "SLEEPAPI_MAX_IN_FLIGHT": "64" if cell["mode"] == "load" else "1",
                       "SLEEPAPI_SHUTDOWN_GRACE_MS": "5000", "RUST_LOG": "sleepapi=warn"}, "server environment")
        require(summary["target_pid"] == 1 and summary["client_pid"] != 1
                and summary["target_comm"] == "sleepapi"
                and summary["target_cmdline"][0] == "/usr/local/bin/sleepapi", "wrong measured process")
        require(re.fullmatch(r"pid:\[[0-9]+\]", summary["pid_namespace"])
                and any(server["id"] in value for value in summary["target_cgroup"]),
                "target namespace/cgroup identity missing")
        require(cpus(summary["target_allowed_cpus"]) == cpus(server["limits"]["CpusetCpus"])
                == cpus(environment["server_cpus"]), "server affinity differs")
        require(cpus(summary["client_allowed_cpus"]) == cpus(worker["limits"]["CpusetCpus"])
                == cpus(environment["client_cpus"]), "client affinity differs")
        require(not cpus(summary["target_allowed_cpus"]) & cpus(summary["client_allowed_cpus"]), "CPU sets overlap")
        cleanup = read_json(directory / "cleanup.json")
        require({c["id"] for c in cleanup} == {server["id"], worker["id"]} and len(cleanup) == 2,
                "cleanup missing an owned container")
        require(all(c["exit_code"] == 0 and c["running"] is False and c["oom_killed"] is False for c in cleanup),
                "unclean or OOM container exit")
        require(all(not c.get("errors") and ("removed" not in c or c["removed"] is True) for c in cleanup),
                "container cleanup failed or removal was not confirmed")
        requests = rows(directory / "data" / "requests.jsonl")
        samples = rows(directory / "data" / "samples.jsonl")
        phases = rows(directory / "data" / "phases.jsonl")
        require(len(requests) == summary["request_records"] and len(samples) == summary["resource_samples"],
                "raw record counts differ")
        for row in requests:
            require(type(row["start_ns"]) is int and type(row["end_ns"]) is int
                    and row["end_ns"] >= row["start_ns"] > 0, "invalid request clock")
            close(row["latency_ms"], (row["end_ns"] - row["start_ns"]) / 1e6, "request latency")
            require(row["route"] == ("/health" if row["phase"] in ("readiness", "overload_health")
                                      else "/sleep/" + cell["method"]), "wrong request route")
            expected_duration = (1500 if row["phase"].endswith("_long") else
                                 100 if row["phase"] in ("warmup", "measured", "overload_burst") else 0)
            require(row["duration_ms"] == expected_duration, "wrong request duration")
            if row["error"] is None:
                headers = row["headers"]
                require(headers["content-type"].split(";", 1)[0].strip().lower() == "application/json"
                        and re.fullmatch(r"[0-9]+", headers["content-length"].strip())
                        and int(headers["content-length"]) <= summary["max_response_body_bytes"],
                        "missing/invalid response framing metadata")
        readiness = one(requests, "phase", "readiness")
        require(readiness["status"] == 200 and readiness["error"] is None
                and readiness["body"] == {"status": "ok"}, "readiness failed")
        for role in ("server", "client"):
            values = [s[role] for s in samples]
            require(values, "missing resource samples")
            start_ticks = summary["process_start_ticks"] if role == "server" else values[0]["process_start_ticks"]
            for sample in values:
                require(all(type(sample[key]) is int and sample[key] >= 0 for key in
                            ("monotonic_ns", "process_start_ticks", "user_cpu_ticks", "system_cpu_ticks",
                             "rss_bytes", "live_threads")), "invalid raw resource counter")
                require(sample["process_start_ticks"] == start_ticks and sample["live_threads"] > 0
                        and sample["rss_bytes"] > 0 and sample["rss_bytes"] % summary["page_size_bytes"] == 0,
                        "invalid or reused measured process")
                close(sample["cpu_seconds"], (sample["user_cpu_ticks"] + sample["system_cpu_ticks"])
                      / summary["clock_ticks_per_second"], "raw CPU seconds")
            require(all(a["monotonic_ns"] < b["monotonic_ns"] and a["user_cpu_ticks"] <= b["user_cpu_ticks"]
                        and a["system_cpu_ticks"] <= b["system_cpu_ticks"] for a, b in zip(values, values[1:])),
                    "sample clock/counters moved backwards")
        calculated = (audit_load if cell["mode"] == "load" else audit_lifecycle)(
            summary, requests, samples, phases, cell)
        audit_rows.append({"case": cell["id"], "mode": cell["mode"], "method": cell["method"],
                           "concurrency": cell["concurrency"], "repetition": cell["repetition"],
                           "request_records": len(requests), "sample_records": len(samples), "recomputed": calculated})
        total_requests += len(requests)
        total_samples += len(samples)
    return {"source_commit": environment["source_commit"], **provenance_result, **compression_result,
            "cases_verified": len(audit_rows),
            "request_records_verified": total_requests, "resource_samples_verified": total_samples,
            "container_identities_verified": len(identities), "rows": audit_rows}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("root", type=Path)
    parser.add_argument("--output", type=Path, required=True, help="new audit receipt; never overwrite")
    args = parser.parse_args()
    root = args.root.resolve()
    output = args.output.resolve()
    report = {"schema_version": 1, "passed": False,
              "checked_at_utc": datetime.now(timezone.utc).isoformat(),
              "verifier_sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
              "scope": "independent raw-data arithmetic, contract, matrix, namespace and cleanup audit"}
    try:
        report.update(audit(root))
        report["sha256"] = {str(path.relative_to(root)): hashlib.sha256(path.read_bytes()).hexdigest()
                            for path in sorted(root.rglob("*")) if path.is_file() and path.resolve() != output}
        report["passed"] = True
    except (AssertionError, OSError, KeyError, ValueError, TypeError, IndexError, ZeroDivisionError,
            tarfile.TarError) as error:
        report["error"] = f"{type(error).__name__}: {error}"
    with output.open("x") as destination:
        json.dump(report, destination, indent=2, allow_nan=False)
        destination.write("\n")
    print(json.dumps({key: report.get(key) for key in
                      ("passed", "cases_verified", "request_records_verified", "resource_samples_verified", "error")}))
    return 0 if report["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())

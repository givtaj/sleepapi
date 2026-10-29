#!/usr/bin/env python3
"""Run isolated Linux resource cells through a local Docker daemon.

Build an immutable SleepAPI image first. The Python worker shares only that
server's PID/network namespaces, never the host namespaces or Docker socket.
"""

import argparse
from datetime import datetime, timezone
import hashlib
import json
from pathlib import Path
import random
import subprocess
import time


def docker(*args, timeout=30):
    result = subprocess.run(["docker", *args], capture_output=True, text=True,
                            timeout=timeout, check=True)
    return result.stdout.strip()


def inspect(reference):
    return json.loads(docker("inspect", reference))[0]


def save(path, value):
    with path.open("x") as output:
        json.dump(value, output, indent=2, allow_nan=False)
        output.write("\n")


def identity(details):
    config = details["HostConfig"]
    return {
        "id": details["Id"], "image": details["Image"],
        "name": details["Name"], "pid": details["State"]["Pid"],
        "started_at": details["State"]["StartedAt"],
        "user": details["Config"]["User"], "command": details["Config"]["Cmd"],
        "environment": [value for value in details["Config"]["Env"]
                        if value.split("=", 1)[0] in ("SLEEPAPI_ADDR", "SLEEPAPI_MAX_IN_FLIGHT",
                                                     "SLEEPAPI_SHUTDOWN_GRACE_MS", "TOKIO_WORKER_THREADS", "RUST_LOG")],
        "limits": {key: config.get(key) for key in
                   ("CpusetCpus", "Memory", "MemorySwap", "PidsLimit", "ReadonlyRootfs")},
        "pid_mode": config["PidMode"], "network_mode": config["NetworkMode"],
    }


def cleanup_owned(created, directory):
    """Try every owned ID and preserve failures before reporting cleanup failure."""
    cleanup = []
    for reference in reversed(created):
        item = {"id": reference, "exit_code": None, "oom_killed": None,
                "running": None, "removed": False}
        errors = []

        def record_error(operation, error):
            errors.append({"operation": operation, "error": type(error).__name__,
                           "message": str(error)})

        def read_state(operation):
            try:
                state = inspect(reference)["State"]
                item.update(exit_code=state["ExitCode"], oom_killed=state.get("OOMKilled"),
                            running=state["Running"])
                return state["Running"]
            except Exception as error:
                record_error(operation, error)
                return None

        running = read_state("inspect_before_stop")
        # If inspection failed, still attempt a bounded stop by the owned ID.
        # Docker stop is also safe for a container that is already stopped.
        if running is not False:
            try:
                docker("stop", "--time", "7", reference, timeout=15)
            except Exception as error:
                record_error("stop", error)
        read_state("inspect_after_stop")
        try:
            # Never force removal: a still-running container must be reported.
            docker("rm", reference)
            item["removed"] = True
        except Exception as error:
            record_error("remove", error)
        if errors:
            item["errors"] = errors
        cleanup.append(item)

    try:
        save(directory / "cleanup.json", cleanup)
    finally:
        directory.chmod(0o755)
    if any(item.get("errors") or item["exit_code"] != 0 or item["oom_killed"]
           or item["running"] is not False or not item["removed"] for item in cleanup):
        raise RuntimeError("owned container cleanup failed; inspect cleanup.json")


def run_case(args, output, image, runner_image, cell, index):
    name = f"sleepapi-bench-{args.run_id}-{index:02d}"
    worker_name = name + "-worker"
    directory = output / cell["id"]
    directory.mkdir()
    # Only this newly created, non-secret evidence directory is writable by the
    # unprivileged Linux worker (UID 65532); parent records provenance afterward.
    directory.chmod(0o777)
    created = []
    try:
        server = docker("run", "-d", "--name", name, "--label", f"sleepapi.benchmark={args.run_id}",
                        "--network", "none", "--cpuset-cpus", args.server_cpus,
                        "--memory", "512m", "--memory-swap", "512m", "--pids-limit", "256",
                        "--read-only", "--cap-drop", "ALL", "--security-opt", "no-new-privileges",
                        "-e", "SLEEPAPI_ADDR=127.0.0.1:3000", "-e", "TOKIO_WORKER_THREADS=2",
                        "-e", "SLEEPAPI_MAX_IN_FLIGHT=" + ("64" if cell["mode"] == "load" else "1"),
                        "-e", "SLEEPAPI_SHUTDOWN_GRACE_MS=5000", "-e", "RUST_LOG=sleepapi=warn", image)
        created.append(server)
        server_info = inspect(server)
        assert server_info["State"]["Running"], "server exited during startup"
        save(directory / "server.json", identity(server_info))
        worker = docker("create", "--name", worker_name,
                        "--label", f"sleepapi.benchmark={args.run_id}",
                        "--network", f"container:{server}", "--pid", f"container:{server}",
                        "--cpuset-cpus", args.client_cpus, "--memory", "512m", "--memory-swap", "512m",
                        "--pids-limit", "256", "--read-only", "--cap-drop", "ALL",
                        "--security-opt", "no-new-privileges", "--user", "65532:65532",
                        "--mount", f"type=bind,src={args.scripts},dst=/work,readonly",
                        "--mount", f"type=bind,src={directory},dst=/out",
                        runner_image, "python3", "-B", "/work/benchmark-worker.py",
                        "--mode", cell["mode"], "--method", cell["method"],
                        "--output-dir", "/out/data", "--window-secs", str(args.window_secs),
                        "--concurrency", str(cell["concurrency"]), "--duration-ms", "100")
        created.append(worker)
        with (directory / "worker.log").open("x") as log:
            process = subprocess.run(["docker", "start", "--attach", worker], stdout=log,
                                     stderr=subprocess.STDOUT, timeout=90)
        worker_info = inspect(worker)
        save(directory / "worker.json", identity(worker_info))
        assert process.returncode == 0 and worker_info["State"]["ExitCode"] == 0, \
            f"worker failed: {directory / 'worker.log'}"
        summary = json.loads((directory / "data" / "summary.json").read_text())
        assert summary["passed"], f"case failed: {cell['id']}"
        after = inspect(server)
        save(directory / "server-after.json", identity(after))
        assert after["State"]["Running"] and not after["State"].get("OOMKilled"), "server stopped or OOMed"
        assert after["State"]["Pid"] == server_info["State"]["Pid"], "server PID changed"
        assert after["State"]["StartedAt"] == server_info["State"]["StartedAt"], "server restarted"
        assert after["RestartCount"] == 0, "unexpected server restart"
        return summary
    finally:
        cleanup_owned(created, directory)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--image", required=True)
    parser.add_argument("--source-sha", required=True)
    parser.add_argument("--output-dir", type=Path, required=True)
    parser.add_argument("--runner-image", default="python:3.12-slim")
    parser.add_argument("--scripts", type=Path, default=Path(__file__).resolve().parent)
    parser.add_argument("--run-id", default="20261029")
    parser.add_argument("--server-cpus", default="2,3")
    parser.add_argument("--client-cpus", default="0,1")
    parser.add_argument("--window-secs", type=float, default=10)
    parser.add_argument("--pilot", action="store_true", help="one load and one lifecycle case per method")
    args = parser.parse_args()
    args.scripts = args.scripts.resolve()
    output = args.output_dir.resolve()
    output.mkdir(parents=True, exist_ok=False)
    image = inspect(args.image)
    runner_image = inspect(args.runner_image)
    revision = image["Config"].get("Labels", {}).get("org.opencontainers.image.revision")
    assert revision == args.source_sha, "image revision label does not match selected source"
    info = json.loads(docker("info", "--format", "{{json .}}"))
    assert info["OSType"] == "linux", "Docker server must be Linux"
    assert not docker("ps", "--quiet"), \
        "start with no running containers for this controlled local benchmark"
    metadata = {
        "schema_version": 1, "started_utc": datetime.now(timezone.utc).isoformat(),
        "source_commit": args.source_sha,
        "image_source_revision": revision,
        "server_image": {key: image.get(key) for key in ("Id", "Architecture", "Os", "Created", "RepoDigests")},
        "runner_image": {key: runner_image.get(key) for key in ("Id", "Architecture", "Os", "Created", "RepoDigests")},
        "binary_sha256": docker("run", "--rm", "--network", "none", "--entrypoint", "sha256sum",
                                image["Id"], "/usr/local/bin/sleepapi").split()[0],
        "docker": {key: info.get(key) for key in
                   ("OperatingSystem", "KernelVersion", "Architecture", "NCPU", "MemTotal", "ServerVersion")},
        "server_cpus": args.server_cpus, "client_cpus": args.client_cpus,
        "server_workers": 2, "request_logging": "disabled (sleepapi=warn)",
        "scope": "Linux containers in Docker Desktop VM; no host-namespace or socket mounts",
        "harness_sha256": {path.name: hashlib.sha256(path.read_bytes()).hexdigest() for path in
                           (Path(__file__).resolve(), args.scripts / "benchmark-worker.py",
                            args.scripts / "benchmark-resources.py")},
    }
    save(output / "environment.json", metadata)
    methods = ["tokio", "tokio-spawn", "thread", "park"]
    cells = []
    for repeat in range(1, 2 if args.pilot else 4):
        group = [(method, concurrency) for method in methods
                 for concurrency in ([16] if args.pilot else [1, 16, 64])]
        random.Random(20261029 + repeat).shuffle(group)
        for method, concurrency in group:
            cells.append({"id": f"load-{method}-c{concurrency}-r{repeat}", "mode": "load",
                          "method": method, "concurrency": concurrency, "repetition": repeat})
        for method in methods:
            cells.append({"id": f"lifecycle-{method}-r{repeat}", "mode": "lifecycle",
                          "method": method, "concurrency": 1, "repetition": repeat})
    save(output / "plan.json", {"window_seconds": args.window_secs, "pilot": args.pilot, "cells": cells})
    results = []
    started = time.monotonic()
    try:
        for index, cell in enumerate(cells, 1):
            print(json.dumps({"event": "start", "case": cell["id"], "index": index, "total": len(cells)}), flush=True)
            summary = run_case(args, output, image["Id"], runner_image["Id"], cell, index)
            results.append({"case": cell, "summary": summary})
            print(json.dumps({"event": "complete", "case": cell["id"], "passed": summary["passed"]}), flush=True)
    finally:
        save(output / "execution.json", {"passed": len(results) == len(cells), "completed": len(results),
                                        "expected": len(cells), "elapsed_seconds": time.monotonic() - started,
                                        "results": results})


if __name__ == "__main__":
    main()

#!/usr/bin/env python3
"""Sampler regressions using a fake /proc tree; no Docker daemon is required."""

import importlib.util
import io
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
import threading
import time
from types import SimpleNamespace
import unittest


SCRIPT = Path(__file__).with_name("benchmark-resources.py")


def fixture_worker(mode):
    spec = importlib.util.spec_from_file_location("benchmark_resources", SCRIPT)
    sampler = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(sampler)
    with tempfile.TemporaryDirectory(prefix="sleepapi-sampler-test-") as directory:
        root = Path(directory)
        pid = 123
        container_id = "a" * 64
        proc = root / str(pid)
        proc.mkdir()
        cgroup = f"0::/docker/{container_id}\n"
        (proc / "cgroup").write_text(cgroup)
        fields = ["0"] * 22
        fields[0], fields[11], fields[12] = "S", "100", "20"
        fields[17], fields[19], fields[21] = "6", "12345", "1024"
        (proc / "stat").write_text(f"{pid} (sleep api) worker) " + " ".join(fields))
        sampler.PROC_ROOT = root
        sampler.inspect_container = lambda _reference: (
            {"Id": container_id, "State": {"Pid": pid}}, cgroup,
        )
        stop_file = root / "stop"
        args = SimpleNamespace(
            container="fixture", interval_ms=10000, max_duration_secs=30,
            stop_file=stop_file if mode == "stop_file" else None,
        )
        handlers = {}
        for signum in (signal.SIGINT, signal.SIGTERM):
            signal.signal(signum, signal.SIG_IGN)
            handlers[signum] = signal.getsignal(signum)

        if mode in ("SIGINT", "SIGTERM", "stop_file"):
            injected = False
            real_sleep = time.sleep
            real_wait = threading.Condition.wait

            def inject():
                nonlocal injected
                if not injected:
                    injected = True
                    if mode == "stop_file":
                        stop_file.write_text("stop\n")
                    else:
                        os.kill(os.getpid(), getattr(signal, mode))

            def sleep(seconds):
                inject()
                real_sleep(seconds)

            def condition_wait(condition, timeout=None):
                # Event.wait holds this non-reentrant lock at entry. The old
                # signal handler deadlocks here when it calls Event.set().
                inject()
                return real_wait(condition, timeout)

            sampler.time.sleep = sleep
            threading.Condition.wait = condition_wait
        elif mode == "max_duration":
            args.max_duration_secs = 0.15
        elif mode == "interval":
            args.interval_ms = 250
            args.max_duration_secs = 0.55
            clock = 1_000_000_000

            def advance(seconds):
                nonlocal clock
                clock += round(seconds * 1_000_000_000)

            sampler.time.monotonic_ns = lambda: clock
            sampler.time.sleep = advance
        elif mode == "inspect_error":
            def fail_inspect(_reference):
                raise ValueError("fixture inspection failure")

            sampler.inspect_container = fail_inspect
        elif mode == "output_error":
            args.max_duration_secs = 0.1

        class Output(io.StringIO):
            def write(self, text):
                if mode == "output_error" and json.loads(text)["type"] == "summary":
                    raise OSError("fixture output failure")
                return super().write(text)

        output = Output()
        error = None
        try:
            code = sampler.run(args, output)
        except OSError as failure:
            code = 1
            error = str(failure)
        print(json.dumps({
            "code": code,
            "error": error,
            "records": [json.loads(line) for line in output.getvalue().splitlines()],
            "handlers_restored": all(
                signal.getsignal(signum) == handler for signum, handler in handlers.items()
            ),
            "stop_file": stop_file.read_text() if stop_file.exists() else None,
        }))


class ResourceSamplerTests(unittest.TestCase):
    def run_fixture(self, mode):
        try:
            completed = subprocess.run(
                [sys.executable, str(Path(__file__).resolve()), "--fixture", mode],
                capture_output=True, text=True, timeout=3, check=True,
            )
        except subprocess.TimeoutExpired:
            self.fail(f"{mode}: sampler hung or failed to stop within three seconds")
        result = json.loads(completed.stdout)
        self.assertTrue(result["handlers_restored"])
        return result

    def assert_summary(self, result, reason):
        records = result["records"]
        summaries = [row for row in records if row["type"] == "summary"]
        self.assertEqual(len(summaries), 1)
        self.assertEqual(records[-1], summaries[0])
        self.assertEqual(summaries[0]["reason"], reason)
        samples = [row for row in records if row["type"] == "sample"]
        self.assertEqual(summaries[0]["sample_count"], len(samples))
        return summaries[0], samples

    def test_signals_at_wait_boundary_do_not_deadlock(self):
        for signum in ("SIGINT", "SIGTERM"):
            with self.subTest(signal=signum):
                result = self.run_fixture(signum)
                self.assertEqual(result["code"], 0)
                summary, samples = self.assert_summary(result, signum)
                self.assertEqual(len(samples), 1)
                self.assertLess(summary["elapsed_seconds"], 1)

    def test_stop_file_interrupts_long_sample_interval(self):
        result = self.run_fixture("stop_file")
        self.assertEqual(result["code"], 0)
        summary, samples = self.assert_summary(result, "stop_file")
        self.assertEqual(len(samples), 1)
        self.assertLess(summary["elapsed_seconds"], 1)
        self.assertEqual(result["stop_file"], "stop\n")

    def test_max_duration_bounds_long_sample_interval(self):
        result = self.run_fixture("max_duration")
        self.assertEqual(result["code"], 0)
        summary, _samples = self.assert_summary(result, "max_duration")
        self.assertGreaterEqual(summary["elapsed_seconds"], 0.15)
        self.assertLess(summary["elapsed_seconds"], 1)

    def test_polling_preserves_sample_interval_and_cpu_metadata(self):
        result = self.run_fixture("interval")
        self.assertEqual(result["code"], 0)
        summary, samples = self.assert_summary(result, "max_duration")
        self.assertEqual([row["elapsed_seconds"] for row in samples], [0, 0.25, 0.5, 0.55])
        self.assertEqual(summary["elapsed_seconds"], 0.55)
        self.assertIsNone(samples[0]["cpu_percent"])
        self.assertTrue(all(row["cpu_percent"] == 0 for row in samples[1:]))
        self.assertEqual([row["sample_index"] for row in samples], [0, 1, 2, 3])
        self.assertTrue(all(row["live_threads"] == 6 for row in samples))

    def test_error_flushes_summary_and_restores_handlers(self):
        result = self.run_fixture("inspect_error")
        self.assertEqual(result["code"], 1)
        _summary, samples = self.assert_summary(result, "error")
        self.assertEqual(samples, [])
        self.assertEqual(result["records"][0]["message"], "fixture inspection failure")

    def test_output_error_still_restores_handlers(self):
        result = self.run_fixture("output_error")
        self.assertEqual(result["code"], 1)
        self.assertEqual(result["error"], "fixture output failure")


if __name__ == "__main__":
    if len(sys.argv) == 3 and sys.argv[1] == "--fixture":
        fixture_worker(sys.argv[2])
    else:
        unittest.main()

"""Sampler shutdown regressions; no Docker daemon or Linux /proc required.

Run with: python3 -B -m unittest discover -s scripts -p 'test_*.py'
"""

from contextlib import contextmanager
import importlib.util
import io
import json
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
import threading
import time
from types import SimpleNamespace
import unittest
from unittest import mock


SAMPLER_PATH = Path(__file__).with_name("benchmark-resources.py")


@contextmanager
def sampler_fixture(interval_ms=100, max_duration_secs=0.25):
    spec = importlib.util.spec_from_file_location("benchmark_resources", SAMPLER_PATH)
    sampler = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(sampler)
    with tempfile.TemporaryDirectory() as temporary:
        root = Path(temporary)
        pid = 123
        container_id = "a" * 64
        cgroup = "0::/docker/" + container_id + "\n"
        (root / str(pid)).mkdir()
        (root / str(pid) / "cgroup").write_text(cgroup)
        details = {"Id": container_id, "State": {"Pid": pid}, "Name": "/fixture"}
        args = SimpleNamespace(
            container="fixture", interval_ms=interval_ms,
            max_duration_secs=max_duration_secs, stop_file=root / "stop",
        )

        def sample(*_args):
            return {
                "monotonic_ns": sampler.time.monotonic_ns(),
                "process_start_ticks": 1, "user_cpu_ticks": 0,
                "system_cpu_ticks": 0, "cpu_seconds": 0,
                "rss_bytes": 4096, "live_threads": 1,
            }

        with mock.patch.object(sampler, "PROC_ROOT", root), \
                mock.patch.object(sampler, "inspect_container", return_value=(details, cgroup)), \
                mock.patch.object(sampler, "process_sample", side_effect=sample):
            yield sampler, args, io.StringIO()


def run_sampler(sampler, args, output):
    originals = {signum: signal.getsignal(signum) for signum in (signal.SIGINT, signal.SIGTERM)}
    exit_code = sampler.run(args, output)
    return {
        "exit_code": exit_code,
        "records": [json.loads(line) for line in output.getvalue().splitlines()],
        "handlers_restored": all(signal.getsignal(signum) == handler for signum, handler in originals.items()),
    }


def signal_worker(signal_name):
    signum = getattr(signal, signal_name)
    original_sleep = time.sleep
    original_wait = threading.Condition.wait
    boundary = None

    def deliver_signal(location):
        nonlocal boundary
        if boundary is None:
            boundary = location
            signal.raise_signal(signum)

    def condition_wait(condition, timeout=None):
        # Event.wait calls this with its Condition lock held. On the original
        # implementation the handler's Event.set deadlocks on that exact lock.
        deliver_signal("condition_wait")
        return original_wait(condition, timeout)

    def sleep(seconds):
        # Also inject at the replacement timer's entry. Sleeping for the full
        # sample interval after the signal would fail the subprocess timeout.
        deliver_signal("sleep")
        original_sleep(seconds)

    with sampler_fixture(interval_ms=10000, max_duration_secs=20) as (sampler, args, output):
        with mock.patch.object(threading.Condition, "wait", condition_wait), \
                mock.patch.object(sampler.time, "sleep", sleep):
            result = run_sampler(sampler, args, output)
    result["signal_boundary"] = boundary
    return result


class FakeClock:
    def __init__(self, on_sleep=None):
        self.now_ns = 1_000_000_000
        self.on_sleep = on_sleep

    def monotonic_ns(self):
        return self.now_ns

    def sleep(self, seconds):
        self.now_ns += round(seconds * 1_000_000_000)
        if self.on_sleep:
            self.on_sleep()


class SamplerShutdownTests(unittest.TestCase):
    def assert_shutdown(self, result, reason):
        self.assertEqual(result["exit_code"], 0)
        self.assertTrue(result["handlers_restored"])
        records = result["records"]
        self.assertEqual(records[0]["type"], "metadata")
        summaries = [record for record in records if record["type"] == "summary"]
        self.assertEqual(len(summaries), 1)
        self.assertIs(summaries[0], records[-1])
        self.assertEqual(summaries[0]["reason"], reason)
        samples = [record for record in records if record["type"] == "sample"]
        self.assertEqual(summaries[0]["sample_count"], len(samples))
        return samples

    def check_signal(self, signal_name):
        # An actual deadlock must fail this test without hanging the test runner.
        child = subprocess.run(
            [sys.executable, "-B", str(Path(__file__).resolve()), "--signal-worker", signal_name],
            capture_output=True, text=True, timeout=3, check=True,
        )
        result = json.loads(child.stdout)
        self.assertIn(result["signal_boundary"], ("condition_wait", "sleep"))
        samples = self.assert_shutdown(result, signal_name)
        self.assertEqual(len(samples), 1)

    def test_sigint_at_wait_boundary(self):
        self.check_signal("SIGINT")

    def test_sigterm_at_wait_boundary(self):
        self.check_signal("SIGTERM")

    def test_duration_deadline_caps_long_sample_interval(self):
        with sampler_fixture(interval_ms=10000, max_duration_secs=0.25) as (sampler, args, output):
            clock = FakeClock()
            with mock.patch.object(sampler, "time", clock):
                result = run_sampler(sampler, args, output)
        samples = self.assert_shutdown(result, "max_duration")
        self.assertEqual([sample["elapsed_seconds"] for sample in samples], [0, 0.25])
        self.assertEqual(result["records"][-1]["elapsed_seconds"], 0.25)

    def test_polling_preserves_sample_intervals(self):
        with sampler_fixture(interval_ms=200, max_duration_secs=0.5) as (sampler, args, output):
            with mock.patch.object(sampler, "time", FakeClock()):
                result = run_sampler(sampler, args, output)
        samples = self.assert_shutdown(result, "max_duration")
        self.assertEqual([sample["elapsed_seconds"] for sample in samples], [0, 0.2, 0.4, 0.5])

    def test_existing_stop_file_finishes_after_first_sample(self):
        with sampler_fixture() as (sampler, args, output):
            args.stop_file.touch()
            result = run_sampler(sampler, args, output)
            self.assertTrue(args.stop_file.exists())
        samples = self.assert_shutdown(result, "stop_file")
        self.assertEqual(len(samples), 1)

    def test_stop_file_created_during_wait(self):
        with sampler_fixture() as (sampler, args, output):
            with mock.patch.object(sampler, "time", FakeClock(on_sleep=args.stop_file.touch)):
                result = run_sampler(sampler, args, output)
            self.assertTrue(args.stop_file.exists())
        samples = self.assert_shutdown(result, "stop_file")
        self.assertEqual(len(samples), 1)


if __name__ == "__main__":
    if len(sys.argv) == 3 and sys.argv[1] == "--signal-worker":
        print(json.dumps(signal_worker(sys.argv[2])))
    else:
        unittest.main()

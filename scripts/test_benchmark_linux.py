#!/usr/bin/env python3
"""Container cleanup regressions; subprocess calls are mocked, no Docker required."""

import importlib.util
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch


SPEC = importlib.util.spec_from_file_location(
    "benchmark_linux", Path(__file__).with_name("benchmark-linux.py")
)
BENCHMARK = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(BENCHMARK)


class CleanupTests(unittest.TestCase):
    def test_worker_cleanup_errors_still_clean_server_and_write_receipt(self):
        server, worker = "a" * 64, "b" * 64
        running = {server: True, worker: True}
        removed = []
        worker_inspections = 0

        def run(command, **kwargs):
            nonlocal worker_inspections
            self.assertEqual(command[0], "docker")
            operation, reference = command[1], command[-1]
            self.assertIn(reference, running)
            if operation == "inspect":
                if reference == worker:
                    worker_inspections += 1
                    if worker_inspections == 1:
                        raise subprocess.CalledProcessError(1, command, stderr="inspection failed")
                state = {"Running": running[reference], "ExitCode": 0, "OOMKilled": False}
                return subprocess.CompletedProcess(command, 0, json.dumps([{"State": state}]))
            if operation == "stop":
                self.assertEqual(kwargs["timeout"], 15)
                if reference == worker:
                    raise subprocess.TimeoutExpired(command, kwargs["timeout"])
                running[reference] = False
            elif operation == "rm":
                if running[reference]:
                    raise subprocess.CalledProcessError(1, command, stderr="container is running")
                removed.append(reference)
            else:
                self.fail(f"unexpected Docker operation: {operation}")
            return subprocess.CompletedProcess(command, 0, reference)

        with tempfile.TemporaryDirectory(prefix="sleepapi-cleanup-test-") as temporary:
            directory = Path(temporary)
            directory.chmod(0o777)
            with patch.object(BENCHMARK.subprocess, "run", side_effect=run):
                with self.assertRaisesRegex(RuntimeError, "cleanup failed"):
                    BENCHMARK.cleanup_owned([server, worker], directory)
            receipt = json.loads((directory / "cleanup.json").read_text())
            self.assertEqual([item["id"] for item in receipt], [worker, server])
            self.assertEqual([error["operation"] for error in receipt[0]["errors"]],
                             ["inspect_before_stop", "stop", "remove"])
            self.assertFalse(receipt[0]["removed"])
            self.assertTrue(receipt[0]["running"])
            self.assertEqual(receipt[1], {"id": server, "exit_code": 0, "oom_killed": False,
                                          "running": False, "removed": True})
            self.assertEqual(removed, [server])
            self.assertEqual(directory.stat().st_mode & 0o777, 0o755)


if __name__ == "__main__":
    unittest.main()

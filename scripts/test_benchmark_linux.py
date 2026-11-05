#!/usr/bin/env python3
"""Container cleanup regressions; subprocess calls are mocked, no Docker required."""

import importlib.util
import json
from pathlib import Path
import subprocess
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch


SPEC = importlib.util.spec_from_file_location(
    "benchmark_linux", Path(__file__).with_name("benchmark-linux.py")
)
BENCHMARK = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(BENCHMARK)


class CreationDaemon:
    """Model daemon state surviving a failed CLI acknowledgement."""

    def __init__(self, failure, reference="b" * 64):
        self.failure = failure
        self.reference = reference
        self.containers = {}
        self.removed = []
        self.name_inspections = 0

    def add(self, reference, name, labels, running=False):
        self.containers[reference] = {
            "Id": reference, "Name": "/" + name, "Config": {"Labels": labels},
            "State": {"Running": running, "ExitCode": 0, "OOMKilled": False}}

    def run(self, command, **kwargs):
        assert command[0] == "docker"
        operation = command[1]
        if operation == "create":
            name = command[command.index("--name") + 1]
            labels = dict(command[i + 1].split("=", 1) for i, value in enumerate(command) if value == "--label")
            if self.failure == "collision":
                self.add(self.reference, name, {BENCHMARK.OWNER_LABEL: "pre-existing-owner"}, running=True)
            else:
                self.add(self.reference, name, labels)
            if self.failure == "start":
                return subprocess.CompletedProcess(command, 0, self.reference)
            if self.failure in ("timeout", "unreachable"):
                raise subprocess.TimeoutExpired(command, kwargs["timeout"])
            raise subprocess.CalledProcessError(1, command, stderr="create acknowledgement failed")
        reference = command[-1]
        if operation == "inspect":
            if reference not in self.containers:
                self.name_inspections += 1
                assert kwargs["timeout"] == 2
                if self.failure == "unreachable" or (self.failure == "timeout" and self.name_inspections == 1):
                    raise subprocess.TimeoutExpired(command, kwargs["timeout"])
                reference = next(key for key, value in self.containers.items() if value["Name"] == "/" + reference)
            return subprocess.CompletedProcess(command, 0, json.dumps([self.containers[reference]]))
        if operation == "start":
            self.containers[reference]["State"]["Running"] = True
            raise subprocess.CalledProcessError(1, command, stderr="start acknowledgement failed")
        if operation == "stop":
            self.containers[reference]["State"]["Running"] = False
        elif operation == "rm":
            assert not self.containers[reference]["State"]["Running"]
            self.removed.append(reference)
            del self.containers[reference]
        else:
            raise AssertionError(f"unexpected Docker operation: {operation}")
        return subprocess.CompletedProcess(command, 0, reference)


class CleanupTests(unittest.TestCase):
    def test_failed_server_and_worker_creation_recovers_only_owned_ids(self):
        for role, failure in (("server", "create"), ("worker", "timeout")):
            with self.subTest(role=role), tempfile.TemporaryDirectory() as temporary:
                directory = Path(temporary)
                daemon = CreationDaemon(failure)
                created = []
                if role == "worker":
                    daemon.add("a" * 64, "owned-server", {}, running=True)
                    created.append("a" * 64)
                with patch.object(BENCHMARK.subprocess, "run", side_effect=daemon.run), \
                        patch.object(BENCHMARK.time, "sleep"):
                    with self.assertRaises(subprocess.SubprocessError):
                        BENCHMARK.create_owned(created, directory, role, "owned-" + role, "image")
                    self.assertIn(daemon.reference, created)
                    BENCHMARK.cleanup_owned(created, directory)
                self.assertFalse(daemon.containers)
                receipt = json.loads((directory / f"{role}-creation.json").read_text())
                self.assertEqual(receipt["status"], "recovered")
                self.assertEqual(receipt["id"], daemon.reference)
                self.assertEqual(daemon.removed, list(reversed(created)))

    def test_preexisting_name_collision_is_never_stopped_or_removed(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            daemon = CreationDaemon("collision")
            created = []
            with patch.object(BENCHMARK.subprocess, "run", side_effect=daemon.run):
                with self.assertRaises(subprocess.CalledProcessError):
                    BENCHMARK.create_owned(created, directory, "server", "occupied-name", "image")
                BENCHMARK.cleanup_owned(created, directory)
            self.assertEqual(created, [])
            self.assertEqual(daemon.removed, [])
            self.assertTrue(daemon.containers[daemon.reference]["State"]["Running"])
            receipt = json.loads((directory / "server-creation.json").read_text())
            self.assertEqual(receipt["status"], "ownership_mismatch")
            self.assertIsNone(receipt["id"])

    def test_unreachable_creation_is_bounded_and_records_unresolved_ownership(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            daemon = CreationDaemon("unreachable")
            created = []
            with patch.object(BENCHMARK.subprocess, "run", side_effect=daemon.run), \
                    patch.object(BENCHMARK.time, "sleep") as sleep:
                with self.assertRaises(subprocess.TimeoutExpired):
                    BENCHMARK.create_owned(created, directory, "server", "uncertain-name", "image")
            self.assertEqual(created, [])
            self.assertEqual(daemon.name_inspections, 3)
            self.assertEqual(sleep.call_count, 2)
            self.assertEqual(daemon.removed, [])
            intent = json.loads((directory / "server-creation-intent.json").read_text())
            receipt = json.loads((directory / "server-creation.json").read_text())
            self.assertEqual(receipt["status"], "unresolved")
            self.assertEqual(receipt["owner_token"], intent["owner_token"])
            self.assertEqual(len(receipt["recovery_errors"]), 3)
            self.assertIn("creation_error", receipt)

    def test_failed_start_after_daemon_launch_still_cleans_server(self):
        with tempfile.TemporaryDirectory() as temporary:
            daemon = CreationDaemon("start")
            args = SimpleNamespace(run_id="start-failure", server_cpus="2,3")
            cell = {"id": "start-failure", "mode": "load"}
            with patch.object(BENCHMARK.subprocess, "run", side_effect=daemon.run):
                with self.assertRaises(subprocess.CalledProcessError):
                    BENCHMARK.run_case(args, Path(temporary), "image", "runner", cell, 1)
            self.assertFalse(daemon.containers)
            self.assertEqual(daemon.removed, [daemon.reference])
            directory = Path(temporary) / cell["id"]
            receipt = json.loads((directory / "cleanup.json").read_text())
            self.assertTrue(receipt[0]["removed"])
            self.assertFalse(receipt[0]["running"])

    def test_binary_probe_start_failure_is_also_cleaned(self):
        with tempfile.TemporaryDirectory() as temporary:
            daemon = CreationDaemon("start")
            with patch.object(BENCHMARK.subprocess, "run", side_effect=daemon.run):
                with self.assertRaises(subprocess.CalledProcessError):
                    BENCHMARK.binary_hash("image", Path(temporary), "probe-failure")
            self.assertFalse(daemon.containers)
            self.assertEqual(daemon.removed, [daemon.reference])
            receipt = json.loads((Path(temporary) / "binary-probe/cleanup.json").read_text())
            self.assertTrue(receipt[0]["removed"])

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

#!/usr/bin/env python3
"""Exercise benchmark ownership against Docker without running a load benchmark."""

import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tempfile
from unittest.mock import patch
import uuid


SPEC = importlib.util.spec_from_file_location(
    "benchmark_linux", Path(__file__).with_name("benchmark-linux.py"))
BENCHMARK = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(BENCHMARK)


def main():
    image = BENCHMARK.inspect(sys.argv[1])["Id"]
    run_id = "ownership-" + uuid.uuid4().hex
    label = f"sleepapi.benchmark={run_id}"
    results = []
    with tempfile.TemporaryDirectory(prefix="sleepapi-ownership-") as temporary:
        root = Path(temporary)
        digest = BENCHMARK.binary_hash(image, root, run_id)
        results.append("binary-probe")

        directory = root / "lost-acknowledgement"
        directory.mkdir()
        created = []
        real_docker = BENCHMARK.docker

        def lose_acknowledgement(*arguments, **kwargs):
            result = real_docker(*arguments, **kwargs)
            if arguments[0] == "create":
                # Docker really created the container; the caller loses its ID.
                raise subprocess.TimeoutExpired(["docker", *arguments], 30)
            return result

        try:
            with patch.object(BENCHMARK, "docker", side_effect=lose_acknowledgement):
                try:
                    BENCHMARK.create_owned(created, directory, "server", run_id + "-lost",
                                           "--label", label, "--network", "none", image)
                except subprocess.TimeoutExpired:
                    pass
                else:
                    raise AssertionError("expected lost create acknowledgement")
            receipt = json.loads((directory / "server-creation.json").read_text())
            assert receipt["status"] == "recovered" and len(created) == 1
        finally:
            BENCHMARK.cleanup_owned(created, directory)
        results.append("lost-create-acknowledgement")

        directory = root / "collision-fixture"
        directory.mkdir()
        created = []
        try:
            fixture = BENCHMARK.create_owned(created, directory, "fixture", run_id + "-collision",
                                             "--label", label, "--network", "none", image)
            rejected = root / "collision-attempt"
            rejected.mkdir()
            unexpected = []
            try:
                try:
                    BENCHMARK.create_owned(unexpected, rejected, "server", run_id + "-collision",
                                           "--label", label, "--network", "none", image)
                except subprocess.CalledProcessError:
                    pass
                else:
                    raise AssertionError("expected name collision")
                assert not unexpected
                assert json.loads((rejected / "server-creation.json").read_text())["status"] == "ownership_mismatch"
                assert BENCHMARK.inspect(fixture)["Id"] == fixture
            finally:
                BENCHMARK.cleanup_owned(unexpected, rejected)
        finally:
            BENCHMARK.cleanup_owned(created, directory)
        results.append("pre-existing-name-collision")

        directory = root / "start-failure"
        directory.mkdir()
        created = []
        try:
            reference = BENCHMARK.create_owned(created, directory, "server", run_id + "-bad-start",
                                               "--label", label, "--network", "none",
                                               "--entrypoint", "/sleepapi-deliberately-missing", image)
            try:
                BENCHMARK.docker("start", reference)
            except subprocess.CalledProcessError:
                pass
            else:
                raise AssertionError("expected invalid entrypoint to fail startup")
        finally:
            try:
                BENCHMARK.cleanup_owned(created, directory)
            except RuntimeError:
                # Docker can report a nonzero exit for the deliberate startup
                # failure. Removal must still succeed and errors stay recorded.
                receipt = json.loads((directory / "cleanup.json").read_text())
                assert receipt and all(r["removed"] and not r["running"] and not r.get("errors") for r in receipt)
        results.append("start-failure")
        assert not BENCHMARK.docker("ps", "-aq", "--filter", "label=" + label), "owned container remains"
    print(json.dumps({"passed": True, "checks": results, "image": image, "binary_sha256": digest}))


if __name__ == "__main__":
    main()

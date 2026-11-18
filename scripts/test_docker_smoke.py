"""Exercise smoke container cleanup with a mocked Docker CLI."""

from contextlib import ExitStack, redirect_stderr, redirect_stdout
import importlib.util
import io
from pathlib import Path
import subprocess
import unittest
from unittest.mock import call, patch


SPEC = importlib.util.spec_from_file_location("docker_smoke", Path(__file__).with_name("docker-smoke.py"))
SMOKE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(SMOKE)
FIRST = "a" * 64
SECOND = "b" * 64


class DockerSmokeCleanupTests(unittest.TestCase):
    def test_run_timeout_recovers_by_name_and_owner_label_before_removal(self):
        containers = {}
        with patch.object(SMOKE, "docker", side_effect=subprocess.TimeoutExpired("run", 20)) as run:
            with self.assertRaises(subprocess.TimeoutExpired):
                SMOKE.start("image", containers)
        (name, (owner, reference)), = containers.items()
        self.assertIsNone(reference)
        label_option = run.call_args.args.index("--label")
        self.assertEqual(run.call_args.args[label_option + 1], f"{SMOKE.OWNER_LABEL}={owner}")
        with patch.object(SMOKE, "docker", side_effect=["", FIRST, FIRST, ""]) as docker:
            with patch.object(SMOKE.time, "sleep"):
                self.assertEqual(SMOKE.cleanup(containers, show_logs=False), [])
        discovery = call("ps", "--all", "--quiet", "--no-trunc", "--filter", f"name=^/{name}$",
                         "--filter", f"label={SMOKE.OWNER_LABEL}={owner}", timeout=2)
        self.assertEqual(docker.call_args_list, [discovery, discovery, call("rm", "--force", FIRST),
                         call("ps", "--all", "--quiet", "--no-trunc", "--filter", f"id={FIRST}")])

    def test_unconfirmed_ownership_skips_logs_and_removal(self):
        with patch.object(SMOKE, "docker", return_value="") as docker, patch.object(SMOKE.time, "sleep"):
            errors = SMOKE.cleanup({"colliding-name": ("new-token", None)}, show_logs=True)
        self.assertEqual(docker.call_count, 3)
        self.assertTrue(all(c.args[0] == "ps" for c in docker.call_args_list))
        self.assertIn("unresolved ownership", errors[0])
        self.assertIn("new-token", errors[0])

    def test_successful_rm_still_requires_confirmed_absence(self):
        with patch.object(SMOKE, "docker", side_effect=[FIRST, FIRST]):
            errors = SMOKE.cleanup({"first": ("one", FIRST)}, show_logs=False)
        self.assertIn("still present", errors[0])

    def test_failed_removal_and_verification_still_attempt_other_containers(self):
        containers = {"first": ("one", FIRST), "second": ("two", SECOND)}
        results = [subprocess.CalledProcessError(1, "rm"), FIRST, SECOND, subprocess.TimeoutExpired("ps", 20)]
        with patch.object(SMOKE, "docker", side_effect=results) as docker:
            errors = SMOKE.cleanup(containers, show_logs=False)
        self.assertEqual(docker.call_args_list, [
            call("rm", "--force", FIRST),
            call("ps", "--all", "--quiet", "--no-trunc", "--filter", f"id={FIRST}"),
            call("rm", "--force", SECOND),
            call("ps", "--all", "--quiet", "--no-trunc", "--filter", f"id={SECOND}"),
        ])
        self.assertIn("removal failed", errors[0])
        self.assertIn("still present", errors[1])
        self.assertIn("could not confirm removal", errors[2])

    def test_main_fails_for_cleanup_and_preserves_an_existing_smoke_failure(self):
        for check_failure in (None, AssertionError("bad response")):
            with self.subTest(check_failure=check_failure), ExitStack() as stack:
                stack.enter_context(patch.object(SMOKE.sys, "argv", ["docker-smoke.py", "image"]))
                stack.enter_context(patch.object(SMOKE, "start", return_value=(FIRST, 3000)))
                for name in ("check_http", "check_recovery_run", "check_fixtures", "check_transport_faults", "stop"):
                    stack.enter_context(patch.object(SMOKE, name))
                stack.enter_context(patch.object(SMOKE, "check_incomplete_body", side_effect=check_failure))
                cleanup = stack.enter_context(patch.object(SMOKE, "cleanup", return_value=["container still present"]))
                stderr = stack.enter_context(redirect_stderr(io.StringIO()))
                stack.enter_context(redirect_stdout(io.StringIO()))
                if check_failure is None:
                    with self.assertRaisesRegex(RuntimeError, "container still present"):
                        SMOKE.main()
                else:
                    with self.assertRaises(AssertionError) as raised:
                        SMOKE.main()
                    self.assertIs(raised.exception, check_failure)
                    self.assertIn("container still present", stderr.getvalue())
                cleanup.assert_called_once_with({}, show_logs=check_failure is not None)


if __name__ == "__main__":
    unittest.main()

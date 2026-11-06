"""Regression checks for publication identity and fail-closed registry guards."""

import hashlib
import importlib.util
import io
import json
from pathlib import Path
import unittest
from unittest.mock import patch
import urllib.error


spec = importlib.util.spec_from_file_location("container_release", Path(__file__).with_name("container-release.py"))
release = importlib.util.module_from_spec(spec)
spec.loader.exec_module(release)


def response(body, headers=None):
    stream = io.BytesIO(body)
    stream.headers = headers or {}
    return stream


class ReleaseGuards(unittest.TestCase):
    def test_tag_must_be_a_version_not_a_ref_or_shell_expression(self):
        self.assertEqual(release.version_from_tag("v0.1.1-alpha.4"), "0.1.1-alpha.4")
        self.assertEqual(release.version_from_tag("v1.2.3"), "1.2.3")
        for tag in ["main", "refs/tags/v1.2.3", "v1.2.3\nother=value", "v1.2.3;echo bad", "v1.2.3/other"]:
            with self.subTest(tag=tag), self.assertRaises(RuntimeError):
                release.version_from_tag(tag)

    def test_only_registry_missing_manifest_is_absence(self):
        for status, code, absent in [(404, "MANIFEST_UNKNOWN", True), (404, "NAME_UNKNOWN", True),
                                     (404, "UNAUTHORIZED", False), (401, "UNAUTHORIZED", False),
                                     (403, "DENIED", False), (500, "UNKNOWN", False)]:
            with self.subTest(status=status, code=code):
                error = urllib.error.HTTPError("https://ghcr.io", status, "failure", {},
                                               io.BytesIO(json.dumps({"errors": [{"code": code}]}).encode()))
                with patch.object(release.urllib.request, "urlopen", side_effect=[response(b'{"token":"test"}'), error]):
                    if absent:
                        self.assertIsNone(release.registry_manifest("0.1.1-alpha.4"))
                    else:
                        with self.assertRaises(urllib.error.HTTPError):
                            release.registry_manifest("0.1.1-alpha.4")

    def test_token_or_network_failure_is_not_absence(self):
        with patch.object(release.urllib.request, "urlopen", side_effect=urllib.error.URLError("offline")):
            with self.assertRaises(urllib.error.URLError):
                release.registry_manifest("0.1.1-alpha.4")

    def test_interrupted_promotion_recovers_without_rewriting_version(self):
        expected = {"amd64": "sha256:a", "arm64": "sha256:b"}
        index = {"manifests": [{"platform": {"os": "linux", "architecture": arch}, "digest": digest}
                               for arch, digest in expected.items()]}
        with patch.object(release, "registry_manifest", return_value=("sha256:existing", index)), \
                patch.object(release.subprocess, "run") as write:
            self.assertEqual(release.publish_index("1.2.3", expected), "sha256:existing")
            write.assert_not_called()
        index['manifests'][1]['digest'] = 'sha256:different'
        with patch.object(release, "registry_manifest", return_value=("sha256:existing", index)), \
                patch.object(release.subprocess, "run") as write:
            with self.assertRaises(RuntimeError):
                release.publish_index("1.2.3", expected)
            write.assert_not_called()

    def test_registry_content_must_match_its_digest(self):
        raw = b'{"schemaVersion":2}'
        digest = "sha256:" + hashlib.sha256(raw).hexdigest()
        for claimed, valid in [(digest, True), ("sha256:wrong", False)]:
            with patch.object(release.urllib.request, "urlopen", side_effect=[
                response(b'{"token":"test"}'), response(raw, {"Docker-Content-Digest": claimed}),
            ]):
                if valid:
                    self.assertEqual(release.registry_manifest("tag")[0], digest)
                else:
                    with self.assertRaises(RuntimeError):
                        release.registry_manifest("tag")

    def test_wrong_manifest_preview_is_rejected_before_publication(self):
        with patch.object(release, "registry_manifest", return_value=None), \
                patch.object(release.subprocess, "check_output", return_value='{"manifests":[]}'), \
                patch.object(release.subprocess, "run") as write:
            with self.assertRaises(RuntimeError):
                release.publish_index("1.2.3", {"amd64": "sha256:a", "arm64": "sha256:b"})
            write.assert_not_called()

    def test_index_requires_both_platforms_and_exact_digests(self):
        expected = {"amd64": "sha256:a", "arm64": "sha256:b"}
        entries = [{"platform": {"os": "linux", "architecture": arch}, "digest": digest}
                   for arch, digest in expected.items()]
        release.check_index({"manifests": entries}, expected)
        for invalid in [entries[:1], entries + entries[:1],
                        [entries[0], {**entries[1], "digest": "sha256:wrong"}]]:
            with self.assertRaises(RuntimeError):
                release.check_index({"manifests": invalid}, expected)

    def test_published_release_requires_complete_ci_on_exact_source(self):
        sha = "a" * 40
        for jobs, valid in [(release.CI_JOBS, True), (release.CI_JOBS - {"Docker / linux/arm64"}, False)]:
            replies = [
                {"draft": False, "published_at": "2026-11-06", "tag_name": "v1.2.3"},
                {"object": {"type": "tag", "sha": "tag-object"}},
                {"object": {"type": "commit", "sha": sha}},
                {"workflow_runs": [{"id": 1, "head_sha": sha, "conclusion": "success"}]},
                {"jobs": [{"name": name, "conclusion": "success"} for name in jobs]},
            ]
            with patch.object(release, "github", side_effect=replies):
                if valid:
                    self.assertEqual(release.release_identity("v1.2.3"), ("1.2.3", sha))
                else:
                    with self.assertRaises(RuntimeError):
                        release.release_identity("v1.2.3")


if __name__ == "__main__":
    unittest.main()

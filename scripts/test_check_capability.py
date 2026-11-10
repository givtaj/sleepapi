#!/usr/bin/env python3
"""Exercise the read-only metadata client against synthetic HTTP servers."""

from contextlib import contextmanager, redirect_stderr, redirect_stdout
from copy import deepcopy
from http.server import BaseHTTPRequestHandler, HTTPServer
import importlib.util
import io
import json
from pathlib import Path
import threading
import unittest
from unittest.mock import patch


SPEC = importlib.util.spec_from_file_location(
    "check_capability", Path(__file__).resolve().parents[1] / "examples" / "check-capability.py"
)
CLIENT = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(CLIENT)
SENTINEL = "PRIVATE_RESPONSE_OR_URL_MUST_NOT_APPEAR"


def metadata():
    return {
        "schema_version": 1,
        "contract_version": 1,
        "software": {"name": "sleepapi", "version": "0.1.1-alpha.4"},
        "capabilities": {key: key != "persistent_runs" for key in CLIENT.CAPABILITIES},
    }


@contextmanager
def server(body=None, status=200, headers=None, declared_length=True):
    body = json.dumps(metadata()).encode() if body is None else body
    requests = []

    class Handler(BaseHTTPRequestHandler):
        def do_GET(self):
            requests.append((self.command, self.path))
            self.send_response(status)
            self.send_header("Content-Type", "application/json")
            if declared_length:
                self.send_header("Content-Length", str(len(body)))
            for key, value in (headers or {}).items():
                self.send_header(key, value)
            self.end_headers()
            try:
                self.wfile.write(body)
            except (BrokenPipeError, ConnectionResetError):
                pass

        def log_message(self, *_args):
            pass

    httpd = HTTPServer(("127.0.0.1", 0), Handler)
    worker = threading.Thread(target=httpd.serve_forever, kwargs={"poll_interval": 0.01})
    worker.start()
    try:
        yield f"http://127.0.0.1:{httpd.server_port}", requests
    finally:
        httpd.shutdown()
        httpd.server_close()
        worker.join(timeout=2)


class CheckCapabilityTests(unittest.TestCase):
    def cli(self, base_url, capability=None):
        args = ["--base-url", base_url]
        if capability is not None:
            args += ["--capability", capability]
        stdout, stderr = io.StringIO(), io.StringIO()
        with redirect_stdout(stdout), redirect_stderr(stderr):
            code = CLIENT.main(args)
        self.assertEqual(stderr.getvalue(), "")
        self.assertLess(len(stdout.getvalue()), 512)
        self.assertNotIn(SENTINEL, stdout.getvalue())
        report = json.loads(stdout.getvalue())
        self.assertIs(report["identity_verified"], False)
        return code, report

    def assert_unknown(self, base_url, capability=None):
        code, report = self.cli(base_url, capability)
        self.assertEqual(code, 2)
        self.assertEqual(report["status"], "unknown")
        self.assertNotIn("declared_software_version", report)

    def test_true_is_supported_and_declared_version_is_unverified(self):
        with server() as (base_url, requests):
            code, report = self.cli(base_url)
        self.assertEqual(code, 0)
        self.assertEqual(report["status"], "supported")
        self.assertEqual(report["capability"], "recovery_runs")
        self.assertEqual(report["declared_software_version"], "0.1.1-alpha.4")
        self.assertEqual(requests, [("GET", "/metadata")])

    def test_false_is_unsupported(self):
        with server() as (base_url, requests):
            code, report = self.cli(base_url, "persistent_runs")
        self.assertEqual(code, 1)
        self.assertEqual(report["status"], "unsupported")
        self.assertEqual(requests, [("GET", "/metadata")])

    def test_old_404_auth_and_server_errors_are_unknown_without_body_leaks(self):
        for status in (404, 401, 403, 500):
            with self.subTest(status=status):
                with server(SENTINEL.encode(), status=status) as (base_url, requests):
                    self.assert_unknown(base_url)
                self.assertEqual(requests, [("GET", "/metadata")])

    def test_redirects_are_not_followed(self):
        for status in (301, 302, 303, 307, 308):
            with self.subTest(status=status):
                with server(SENTINEL.encode(), status=status,
                            headers={"Location": "/" + SENTINEL}) as (base_url, requests):
                    self.assert_unknown(base_url)
                self.assertEqual(requests, [("GET", "/metadata")])

    def test_invalid_metadata_is_unknown_even_if_requested_capability_is_true(self):
        changes = [
            ("missing schema", lambda d: d.pop("schema_version")),
            ("new schema", lambda d: d.update(schema_version=2)),
            ("boolean schema", lambda d: d.update(schema_version=True)),
            ("float schema", lambda d: d.update(schema_version=1.0)),
            ("missing contract", lambda d: d.pop("contract_version")),
            ("unknown contract", lambda d: d.update(contract_version=2)),
            ("missing software", lambda d: d.pop("software")),
            ("wrong software", lambda d: d["software"].update(name="another-server")),
            ("empty version", lambda d: d["software"].update(version="")),
            ("invalid version", lambda d: d["software"].update(version=SENTINEL)),
            ("oversized version", lambda d: d["software"].update(version="1.0.0-" + "a" * 129)),
            ("leading zero", lambda d: d["software"].update(version="1.0.0-01")),
            ("missing capabilities", lambda d: d.pop("capabilities")),
            ("missing unrelated key", lambda d: d["capabilities"].pop("delays")),
            ("integer capability", lambda d: d["capabilities"].update(recovery_runs=1)),
            ("string capability", lambda d: d["capabilities"].update(recovery_runs="true")),
            ("null capability", lambda d: d["capabilities"].update(recovery_runs=None)),
            ("invalid added capability", lambda d: d["capabilities"].update(unrecognized="true")),
        ]
        for name, change in changes:
            with self.subTest(case=name):
                value = deepcopy(metadata())
                change(value)
                with server(json.dumps(value).encode()) as (base_url, _requests):
                    self.assert_unknown(base_url)

    def test_additive_fields_and_boolean_capabilities_are_accepted(self):
        value = metadata()
        value["new_metadata"] = SENTINEL
        value["software"]["extra"] = SENTINEL
        value["capabilities"]["future_capability"] = True
        with server(json.dumps(value).encode()) as (base_url, requests):
            code, report = self.cli(base_url)
            self.assert_unknown(base_url, "future_capability")
        self.assertEqual((code, report["status"]), (0, "supported"))
        self.assertEqual(requests, [("GET", "/metadata")])

    def test_malformed_duplicate_and_nonstandard_json_are_unknown(self):
        valid = json.dumps(metadata()).encode()
        for body in (SENTINEL.encode(), b"[]", b"null", b"\xff", valid[:-1],
                     valid.replace(b'"schema_version": 1', b'"schema_version": 2, "schema_version": 1'),
                     valid.replace(b'"schema_version": 1', b'"schema_version": NaN')):
            with self.subTest(body=body[:20]):
                with server(body) as (base_url, _requests):
                    self.assert_unknown(base_url)

    def test_oversize_is_unknown_with_or_without_content_length(self):
        body = json.dumps(metadata()).encode().ljust(CLIENT.MAX_BODY_BYTES + 1)
        for declared_length in (True, False):
            with self.subTest(declared_length=declared_length):
                with server(body, declared_length=declared_length) as (base_url, _requests):
                    self.assert_unknown(base_url)

    def test_exact_body_limit_is_accepted(self):
        body = json.dumps(metadata()).encode().ljust(CLIENT.MAX_BODY_BYTES)
        with server(body) as (base_url, _requests):
            code, report = self.cli(base_url)
        self.assertEqual((code, report["status"]), (0, "supported"))

    def test_ambiguous_or_truncated_framing_is_unknown(self):
        for headers, automatic_length in (({"Content-Length": "5"}, True),
                                          ({"Content-Length": "8192"}, False),
                                          ({"Content-Length": "-1"}, False),
                                          ({"Transfer-Encoding": "chunked"}, True),
                                          ({"Transfer-Encoding": "unsupported"}, False)):
            with self.subTest(headers=headers):
                with server(headers=headers, declared_length=automatic_length) as (base_url, _requests):
                    self.assert_unknown(base_url)

    def test_network_errors_are_unknown_without_exception_details(self):
        for error in (TimeoutError(SENTINEL), ConnectionRefusedError(SENTINEL),
                      CLIENT.http.client.BadStatusLine(SENTINEL)):
            with self.subTest(error=type(error).__name__):
                with patch.object(CLIENT.http.client, "HTTPConnection") as connection:
                    connection.return_value.request.side_effect = error
                    self.assert_unknown("http://127.0.0.1:3000")
                    self.assertEqual(connection.call_args.kwargs["timeout"], 2)
                    connection.return_value.close.assert_called_once()

    def test_invalid_addresses_and_capabilities_make_no_request(self):
        for base_url in ("https://127.0.0.1", "http://example.com", "http://192.0.2.1",
                         "http://127.0.0.1/" + SENTINEL, "http://" + SENTINEL + "@127.0.0.1",
                         "http://127.0.0.1?" + SENTINEL, "http://127.0.0.1#" + SENTINEL,
                         "http://127.0.0.1:0", "http://127.0.0.1:65536", "http://[::1",
                         "http://127.0.0.1/\n" + SENTINEL):
            with self.subTest(base_url=base_url):
                with patch.object(CLIENT.http.client, "HTTPConnection") as connection:
                    self.assert_unknown(base_url)
                    connection.assert_not_called()
        with patch.object(CLIENT.http.client, "HTTPConnection") as connection:
            self.assert_unknown("http://127.0.0.1", SENTINEL)
            connection.assert_not_called()

    def test_localhost_and_ipv6_stay_on_loopback(self):
        self.assertEqual(CLIENT.local_address("http://localhost:3000/"), ("127.0.0.1", 3000))
        self.assertEqual(CLIENT.local_address("http://[::1]:3000"), ("::1", 3000))

    def test_argument_errors_do_not_echo_input(self):
        stdout, stderr = io.StringIO(), io.StringIO()
        with redirect_stdout(stdout), redirect_stderr(stderr):
            with self.assertRaises(SystemExit) as error:
                CLIENT.main(["--" + SENTINEL])
        self.assertEqual(error.exception.code, 2)
        self.assertEqual(stderr.getvalue(), "")
        self.assertEqual(json.loads(stdout.getvalue()), {"status": "unknown", "identity_verified": False})


if __name__ == "__main__":
    unittest.main()

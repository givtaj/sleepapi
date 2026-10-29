#!/usr/bin/env python3
"""Reject false-positive fault classifications in the runnable retry example."""

import http.client
import importlib.util
import io
from pathlib import Path
import unittest
from unittest.mock import patch


SPEC = importlib.util.spec_from_file_location(
    "retry_client", Path(__file__).resolve().parents[1] / "examples" / "retry-client.py"
)
CLIENT = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(CLIENT)


class Socket:
    def __init__(self, wire):
        self.wire = wire

    def makefile(self, _mode):
        return io.BytesIO(self.wire)


class Connection:
    """Use the real HTTP parser with a deterministic, already-closed wire stream."""

    def __init__(self, wire):
        self.wire = wire

    def request(self, *_args):
        pass

    def getresponse(self):
        response = http.client.HTTPResponse(Socket(self.wire))
        response.begin()
        return response

    def close(self):
        pass


class RetryClientTests(unittest.TestCase):
    def exchange(self, wire):
        with patch.object(CLIENT.http.client, "HTTPConnection", return_value=Connection(wire)):
            return CLIENT.exchange(("127.0.0.1", 3000), "GET", "/")

    def test_incomplete_body_preserves_exact_partial_bytes(self):
        result = self.exchange(b"HTTP/1.1 200 OK\r\nContent-Length: 6\r\n\r\na\xc3")
        self.assertEqual(result["kind"], "truncated_response")
        self.assertEqual(result["body"], b"a\xc3")
        self.assertEqual(result["declared_body_bytes"], 6)

    def test_only_absent_headers_count_as_connection_drop(self):
        self.assertEqual(self.exchange(b"")["kind"], "connection_drop")
        with self.assertRaises(http.client.BadStatusLine):
            self.exchange(b"not HTTP\r\n")

    def test_timeout_and_refused_connection_are_not_expected_faults(self):
        for error in (TimeoutError("timed out"), ConnectionRefusedError("not listening")):
            with self.subTest(error=type(error).__name__):
                with patch.object(CLIENT.http.client, "HTTPConnection") as factory:
                    factory.return_value.getresponse.side_effect = error
                    with self.assertRaises(type(error)):
                        CLIENT.exchange(("127.0.0.1", 3000), "POST", "/runs/run-1/attempts", {})

    def test_unbounded_ambiguous_and_oversized_framing_fail(self):
        for headers in (b"", b"Content-Length: 65537\r\n", b"Content-Length: 1\r\nContent-Length: 2\r\n",
                        b"Transfer-Encoding: chunked\r\nContent-Length: 1\r\n"):
            with self.subTest(headers=headers):
                with self.assertRaises(AssertionError):
                    self.exchange(b"HTTP/1.1 200 OK\r\n" + headers + b"\r\n")

    def test_valid_status_with_wrong_fixture_does_not_retry(self):
        sequence = CLIENT.cases()[0][1]
        expected = sequence[0]["fixture"]
        wrong = expected["body"].replace("false", "FALSE").encode()
        wire = (b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: " + str(len(wrong)).encode()
                + b"\r\nContent-Type: application/json; charset=utf-8\r\nRetry-After: 0"
                + b"\r\nX-Request-ID: retry-client\r\n\r\n" + wrong)
        observations = []
        with patch.object(CLIENT.http.client, "HTTPConnection", return_value=Connection(wire)) as factory:
            with self.assertRaisesRegex(AssertionError, "fixture bytes differ"):
                CLIENT.retry_sequence(("127.0.0.1", 3000), "/runs/run-1", 1, sequence, observations)
            self.assertEqual(factory.call_count, 1)
        self.assertEqual(len(observations), 1)


if __name__ == "__main__":
    unittest.main()

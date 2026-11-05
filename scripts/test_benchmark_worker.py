#!/usr/bin/env python3
"""Check benchmark arithmetic and response accounting without Docker or traffic."""

import importlib.util
import http.client
import io
from pathlib import Path
import unittest


SPEC = importlib.util.spec_from_file_location(
    "benchmark_worker", Path(__file__).with_name("benchmark-worker.py"))
WORKER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(WORKER)


class BenchmarkWorkerTests(unittest.TestCase):
    def sample(self, ns, ticks, rss=4096, threads=6):
        return {"monotonic_ns": ns, "user_cpu_ticks": ticks, "system_cpu_ticks": 0,
                "rss_bytes": rss, "live_threads": threads}

    def test_cpu_uses_only_samples_strictly_inside_phase(self):
        samples = [self.sample(0, 0, 99999, 99), self.sample(1_100_000_000, 100),
                   self.sample(1_500_000_000, 110, 8192, 8), self.sample(1_900_000_000, 120),
                   self.sample(3_000_000_000, 999, 99999, 99)]
        result = WORKER.resource_summary(samples, 1_000_000_000, 2_000_000_000, 100)
        self.assertEqual(result["sample_count"], 3)
        self.assertAlmostEqual(result["cpu_percent_one_core"], 25)
        self.assertEqual(result["rss_peak_bytes"], 8192)
        self.assertEqual(result["threads_peak"], 8)
        self.assertEqual(result["leading_unsampled_ms"], 100)
        self.assertEqual(result["trailing_unsampled_ms"], 100)
        self.assertEqual(result["max_sample_gap_ms"], 400)

    def test_cpu_rejects_insufficient_samples_or_reversed_counters(self):
        with self.assertRaises(AssertionError):
            WORKER.resource_summary([self.sample(5, 1)], 0, 10, 100)
        with self.assertRaises(AssertionError):
            WORKER.resource_summary([self.sample(1, 2), self.sample(2, 1)], 0, 10, 100)

    def test_percentiles_use_nearest_rank_and_preserve_outliers(self):
        values = list(range(1, 101))
        self.assertEqual(WORKER.percentile(values, .5), 50)
        self.assertEqual(WORKER.percentile(values, .95), 95)
        with self.assertRaises(AssertionError):
            WORKER.percentile([], .95)

    def test_short_valid_json_is_not_a_complete_response(self):
        class Socket:
            def makefile(self, _mode):
                return io.BytesIO(b'HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 20\r\n\r\n{"status":"ok"}')

        class Connection:
            def request(self, *_args):
                pass

            def getresponse(self):
                response = http.client.HTTPResponse(Socket())
                response.begin()
                return response

        row = WORKER.observe(Connection(), "tokio", 0, "test", path="/health")
        self.assertEqual(row["error"]["type"], "IncompleteRead")
        self.assertIsNone(row["body"])

    def valid_response(self):
        return {"error": None, "status": 200, "latency_ms": 105,
                "body": {"status_code": 200, "simulated": True, "operation": "sleep",
                         "method": "tokio_sleep", "blocks_thread": False,
                         "requested_duration_ms": 100, "elapsed_ms": 101}}

    def test_measured_success_rejects_wrong_method_early_or_nonfinite_results(self):
        WORKER.validate_success(self.valid_response(), "tokio", 100)
        for field, value in [("method", "thread_sleep"), ("elapsed_ms", 99),
                             ("elapsed_ms", float("nan")), ("simulated", False),
                             ("requested_duration_ms", 10), ("status_code", 503)]:
            with self.subTest(field=field, value=value):
                row = self.valid_response()
                row["body"][field] = value
                with self.assertRaises(AssertionError):
                    WORKER.validate_success(row, "tokio", 100)
        row = self.valid_response()
        row["latency_ms"] = 90
        with self.assertRaises(AssertionError):
            WORKER.validate_success(row, "tokio", 100)

    def test_simulated_rate_limiting_is_not_capacity_evidence(self):
        row = {"error": None, "status": 429, "headers": {"retry-after": "1"},
               "body": {"simulated": False, "status_code": 429,
                        "error": {"code": "capacity_exceeded"}}}
        WORKER.validate_capacity(row)
        row["body"]["simulated"] = True
        with self.assertRaises(AssertionError):
            WORKER.validate_capacity(row)
        row["body"]["simulated"] = False
        row["body"]["error"]["code"] = "rate_limited"
        with self.assertRaises(AssertionError):
            WORKER.validate_capacity(row)


if __name__ == "__main__":
    unittest.main()

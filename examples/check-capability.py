#!/usr/bin/env python3
"""Read one capability from a local SleepAPI server using Python 3.9+.

GET /metadata is read-only. No redirects, retries, server startup, or lifecycle
operations are performed. Exit 0 means supported, 1 unsupported, and 2 unknown.
The declared software version is self-reported, never verified provenance.
"""

import argparse
import http.client
import ipaddress
import json
import re
from urllib.parse import urlsplit


TIMEOUT_SECONDS = 2
MAX_BODY_BYTES = 8192
CAPABILITIES = frozenset({
    "delays", "status_codes", "recovery_runs", "run_reset", "run_receipts",
    "response_fixtures", "connection_drop", "truncated_response", "persistent_runs",
})
OPTIONAL_CAPABILITIES = frozenset({"accept_then_drop"})
EXIT_CODES = {"supported": 0, "unsupported": 1, "unknown": 2}
VERSION = re.compile(
    r"(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)"
    r"(?:-([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?"
    r"(?:\+[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?"
)


def local_address(base_url):
    """Accept only HTTP loopback URLs without credentials or extra URL parts."""
    if any(character.isspace() or ord(character) < 32 for character in base_url):
        raise ValueError("invalid base URL")
    url = urlsplit(base_url)
    if (url.scheme != "http" or not url.hostname
            or url.username is not None or url.password is not None
            or url.path not in ("", "/") or url.query or url.fragment
            or "%" in url.hostname):
        raise ValueError("invalid base URL")
    host = "127.0.0.1" if url.hostname == "localhost" else url.hostname
    if not ipaddress.ip_address(host).is_loopback:
        raise ValueError("invalid base URL")
    port = url.port if url.port is not None else 80
    if not 1 <= port <= 65535:
        raise ValueError("invalid base URL")
    return host, port


def unique_object(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError("duplicate JSON field")
        result[key] = value
    return result


def reject_constant(_value):
    raise ValueError("invalid JSON constant")


def valid_names(value):
    return (isinstance(value, list) and len(value) > 0
            and all(isinstance(name, str) and name for name in value))


def positive_limits(value, fields):
    return (isinstance(value, dict)
            and all(type(value.get(field)) is int and value[field] > 0 for field in fields))


def valid_limits(limits):
    if not positive_limits(limits, ("max_duration_ms", "max_in_flight", "max_request_body_bytes")):
        return False
    runs = limits.get("runs")
    if not positive_limits(runs, ("max_runs", "max_sequence_steps", "max_attempts_per_run",
                                  "max_receipts_per_run", "default_ttl_ms", "max_ttl_ms")):
        return False
    fixtures = limits.get("fixtures")
    return (runs["default_ttl_ms"] <= runs["max_ttl_ms"]
            and positive_limits(fixtures, ("max_body_bytes", "max_headers", "max_header_value_bytes"))
            and valid_names(fixtures.get("content_types"))
            and valid_names(fixtures.get("header_names")))


def valid_metadata(metadata):
    if not isinstance(metadata, dict):
        return False
    for field in ("schema_version", "contract_version"):
        if type(metadata.get(field)) is not int or metadata[field] != 1:
            return False
    software = metadata.get("software")
    capabilities = metadata.get("capabilities")
    if not isinstance(software, dict) or software.get("name") != "sleepapi":
        return False
    version = software.get("version")
    if not isinstance(version, str) or len(version) > 128:
        return False
    match = VERSION.fullmatch(version)
    if match is None:
        return False
    if match[4] and any(part.isdigit() and len(part) > 1 and part[0] == "0"
                        for part in match[4].split(".")):
        return False
    if (not isinstance(capabilities, dict) or not CAPABILITIES <= capabilities.keys()
            or not all(type(value) is bool for value in capabilities.values())):
        return False
    protocols = metadata.get("protocols")
    return (isinstance(protocols, dict) and valid_names(protocols.get("transport_faults"))
            and "HTTP/1.1" in protocols["transport_faults"]
            and valid_names(metadata.get("wait_methods"))
            and valid_names(metadata.get("scenarios"))
            and valid_limits(metadata.get("limits")))


def check_capability(base_url, capability="recovery_runs"):
    report = {"status": "unknown", "identity_verified": False}
    if capability not in CAPABILITIES and capability not in OPTIONAL_CAPABILITIES:
        return report
    report["capability"] = capability
    connection = None
    try:
        connection = http.client.HTTPConnection(*local_address(base_url), timeout=TIMEOUT_SECONDS)
        connection.request("GET", "/metadata", headers={"Accept": "application/json"})
        response = connection.getresponse()
        # Error and redirect bodies are neither read nor included in output.
        if response.status != 200:
            return report
        lengths = response.headers.get_all("Content-Length", [])
        if len(lengths) > 1 or (lengths and not lengths[0].isascii()):
            return report
        if lengths and (not lengths[0].isdigit() or int(lengths[0]) > MAX_BODY_BYTES):
            return report
        if lengths and response.getheader("Transfer-Encoding") is not None:
            return report
        if response.getheader("Transfer-Encoding") not in (None, "chunked"):
            return report
        body = response.read(MAX_BODY_BYTES + 1)
        if len(body) > MAX_BODY_BYTES or (lengths and len(body) != int(lengths[0])):
            return report
        metadata = json.loads(body.decode("utf-8"), object_pairs_hook=unique_object,
                              parse_constant=reject_constant)
        if valid_metadata(metadata) and capability in metadata["capabilities"]:
            report["status"] = "supported" if metadata["capabilities"][capability] else "unsupported"
            report["declared_software_version"] = metadata["software"]["version"]
    except (OSError, ValueError, http.client.HTTPException, RecursionError):
        # Exception messages can contain URLs or server content. Emit only the
        # fixed classification, without remote bodies, headers, or environment.
        pass
    finally:
        if connection is not None:
            connection.close()
    return report


class SafeArgumentParser(argparse.ArgumentParser):
    def error(self, _message):
        print('{"status":"unknown","identity_verified":false}')
        self.exit(2)


def main(argv=None):
    parser = SafeArgumentParser(description=__doc__)
    parser.add_argument("--base-url", required=True, help="HTTP localhost or loopback IP URL")
    parser.add_argument("--capability", default="recovery_runs",
                        help="capability to inspect (default: recovery_runs)")
    args = parser.parse_args(argv)
    report = check_capability(args.base_url, args.capability)
    print(json.dumps(report, sort_keys=True))
    return EXIT_CODES[report["status"]]


if __name__ == "__main__":
    raise SystemExit(main())

# Server metadata and capability checks

`GET /metadata` lets a client discover capabilities and configured limits before
starting a test. It returns `200` JSON with `Cache-Control: no-store`, without
consuming simulation capacity, creating a run, or changing existing runs.

This endpoint and its example client are included in the **unreleased
`0.1.1-alpha.5` source candidate**. The
published `v0.1.1-alpha.4` source tag does not contain `/metadata`.
Build the candidate checkout to try it; its declared Cargo package version is
`0.1.1-alpha.5`. No alpha.5 tag or release is published yet. Distribution is
source plus a Dockerfile for local builds; new registry images will not be published.
Use explicit capability discovery rather than inferring support from a version
string, and verify artifact provenance separately.

## Try the current source

From a checkout containing this guide, start the server in one terminal:

```sh
cargo run --locked
```

From a second terminal in the same checkout:

```sh
curl --http1.1 -i http://127.0.0.1:3000/metadata
python3 examples/check-capability.py \
  --base-url http://127.0.0.1:3000 --capability recovery_runs
```

The Python example requires Python 3.9 or newer and uses only its standard
library. It reads metadata from an existing loopback server without following
redirects. It does not start or stop the server, create or reset runs, send
simulation requests, or inject faults. Stop the server with Ctrl-C when finished.

The example prints a JSON result with one of these `status` values:

| Status | Meaning | Exit code |
| --- | --- | --- |
| `supported` | A recognized metadata response explicitly reports the requested capability as `true` | `0` |
| `unsupported` | A recognized metadata response explicitly reports the requested capability as `false` | `1` |
| `unknown` | Support could not be established | `2` |

For an explicit unsupported result, check `--capability persistent_runs`.
An older server returning `404`, an unrecognized or missing schema or contract
version, a missing capability, malformed metadata, or a failed request produces `unknown`, not
`unsupported`. A missing discovery endpoint does not prove that the older
server lacks recovery runs or another feature.

The result always includes `identity_verified: false`. Metadata is a server's
self-description; this check does not authenticate its identity or artifact.

## Response contract

The current response has these top-level fields:

| Field | Meaning |
| --- | --- |
| `schema_version` | Integer `1`; version of the metadata response structure and interpretation |
| `contract_version` | Integer `1`; SleepAPI HTTP contract revision described by this metadata |
| `software` | Object with `name: "sleepapi"` and the build's Cargo package `version` |
| `capabilities` | Object mapping capability names to explicit booleans |
| `protocols` | Object with `transport_faults: ["HTTP/1.1"]` |
| `wait_methods` | Supported waiting-method names |
| `scenarios` | Supported scenario names |
| `limits` | Configured shared limits and fixed run/fixture limits, described below |

`contract_version` describes the contract advertised by the responding build.
It does not establish support in older releases that lack this metadata,
guarantee that every advertised capability is available over every protocol,
or authenticate a binary, container, source revision, or release. Use explicit
capabilities and limits to decide whether to attempt a test; use release
artifacts and their verification records separately when provenance matters.

Clients must accept unknown fields and capability names. Additive fields and
capabilities can be introduced within schema version `1`; incompatible changes
to existing field types or meanings require a schema-version change. A client
that does not understand the schema must treat capability support as unknown.

### Capabilities and protocols

| Capability | Current value and scope |
| --- | --- |
| `delays` | `true`; configurable response waits |
| `status_codes` | `true`; simulated statuses subject to the [HTTP contract](../README.md#http-contract) |
| `recovery_runs` | `true`; isolated finite response sequences |
| `run_reset` | `true`; generation-guarded reset/replay |
| `run_receipts` | `true`; bounded per-run diagnostic receipts |
| `response_fixtures` | `true`; exact UTF-8 fixture bodies within the fixture limits |
| `connection_drop` | `true` only when this metadata request uses HTTP/1.1 through the instrumented `FaultListener` |
| `truncated_response` | `true` only when this metadata request uses HTTP/1.1 through the instrumented `FaultListener` |
| `persistent_runs` | `false`; runs and receipts are process-local and lost on restart |

Transport-fault capability values describe the path used by the metadata
request. A bare embedded router without the instrumented listener reports
`false`, even though the same software can inject faults when served through
that listener. `protocols.transport_faults` names the supported fault protocol;
it is not a list of every protocol the server might accept. Later fault
requests must also use HTTP/1.1 through the instrumented listener. A proxy may
change the connection or buffer responses, so metadata alone cannot prove the
fault that a downstream client will observe.

`wait_methods` currently contains `tokio_sleep`, `tokio_spawn`, `thread_sleep`,
and `thread_park`. `scenarios` contains `fast_success`, `slow_provider`,
`delayed_failure`, `client_timeout`, `rate_limited`, `invalid_request`, and
`upstream_timeout`. See [waiting methods](../README.md#waiting-methods) and
[scenarios](../README.md#scenarios) for their behavior.

### Limits

`limits.max_duration_ms` and `limits.max_in_flight` report the responding
server's configured values, rather than assuming the defaults of `30000` ms and
`64` active waits. `limits.max_request_body_bytes` is `8192`.

`limits.runs` contains:

| Field | Current limit |
| --- | --- |
| `max_runs` | `64` |
| `max_sequence_steps` | `64` |
| `max_attempts_per_run` | `1024` across all generations |
| `max_receipts_per_run` | `32` |
| `default_ttl_ms` | `300000` |
| `max_ttl_ms` | `3600000` |

`limits.fixtures` contains `max_body_bytes: 4096`, `max_headers: 5`, and
`max_header_value_bytes: 256`, plus the `content_types` and `header_names`
allowlists. Body bytes are decoded UTF-8 bytes; header values must be printable
ASCII. The whole run-creation request still has to fit the shared request-body
limit. `content_types` lists the base media types `application/json`,
`text/plain`, and `application/xml`; each also accepts the optional
`; charset=utf-8` suffix. `header_names` lists `retry-after`, `www-authenticate`,
`allow`, `cache-control`, and `x-request-id`. See [recovery runs](recovery-runs.md) and
[response fixtures](response-fixtures.md) for validation and lifecycle rules.

## Read-only scope

The endpoint reports capabilities and limits without live run counts, run
identifiers, sequences, receipts, or payloads. Reading it does not prune expired
runs or reserve capacity. It does not dump environment variables or expose
listen addresses, CORS origins, or filesystem paths. Health checks remain
available separately at `/health`.

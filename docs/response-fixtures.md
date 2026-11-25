# Response fixtures and transport faults

Response fixtures, `connection_drop`, and `truncated_response` are included from
**`v0.1.1-alpha.4`**. See the [prerelease notes](https://github.com/givtaj/sleepapi/releases/tag/v0.1.1-alpha.4)
and the [recovery run contract](recovery-runs.md). `accept_then_drop` is available
in the **unreleased `0.1.1-alpha.6` candidate**. Final tagged-source installation
and publication verification remain pending; the quickstart still uses alpha.5.

## Exact fixture responses

An optional `fixture` on a sequence step replaces the simulation JSON body with
the exact UTF-8 bytes of its decoded `body` string. JSON whitespace, newlines,
and Unicode are preserved; SleepAPI does not parse, reformat, or wrap that body.
Clients still initiate every attempt and supply the current generation.

```sh
curl -sS http://127.0.0.1:3000/runs \
  -H 'Content-Type: application/json' \
  -d '{"sequence":[{"status_code":503,"duration_ms":50,"fixture":{"body":"{ \"ready\": false }\n","content_type":"application/json","headers":{"retry-after":"3","x-request-id":"example"}}},{"status_code":200,"duration_ms":10,"fixture":{"body":"ready\n","content_type":"text/plain"}}]}'
```

Use the returned `run_id` and `generation` with
`POST /runs/{run_id}/attempts`. The first response has status `503`, a 50 ms
requested wait, and the exact body `{ "ready": false }` followed by a newline.
It includes `Retry-After: 3`. The second returns `ready` followed by a newline.
Omitting `fixture` preserves the existing simulation JSON response.

| Field | Contract |
| --- | --- |
| `body` | Required UTF-8 JSON string, including empty strings; at most 4,096 decoded bytes. Binary encoding is not supported. |
| `content_type` | Defaults to `application/json`. Accepts `application/json`, `text/plain`, or `application/xml`, each optionally followed by exactly `; charset=utf-8`. The supplied allowed value is emitted unchanged. |
| `headers` | Optional object with at most five values. Names are case-insensitive; duplicate normalized names are rejected. Each value is at most 256 bytes of printable ASCII. |

The header allowlist is `retry-after`, `www-authenticate`, `allow`, `cache-control`,
and `x-request-id`. Fixture overrides replace the simulation defaults for
`Retry-After`, `WWW-Authenticate`, and `Allow`. Framing headers such as
`Content-Length`, `Transfer-Encoding`, and `Connection`, and CORS headers, cannot
be supplied. `Content-Type` is selected only through `content_type`.
The usual supported-status rules remain in force, including rejection of
bodyless, range, proxy-authentication, and protocol-upgrade statuses.

Validation happens before a run is created; invalid fixtures return a structured
`422`. The existing **8 KiB JSON request limit applies to the entire run**,
including JSON escaping and all fixtures; oversized requests return `413`.
This may prevent combining multiple maximum-size fixtures in one request.

## Diagnostics and lifecycle

Read `GET /runs/{run_id}/receipts` for generation, attempt, selected status,
waiting method, requested delay, server elapsed time, and outcome. These values
are never appended to fixture bodies. Run inspection also retains the configured
steps. Timing describes server processing, not client-observed latency.

There are at most 64 steps per run, 64 live runs, 1,024 lifetime attempts per
run, and 32 recent receipts per run. The full request limit bounds each run's
configured fixture data. Reset keeps the fixtures and original TTL while
rewinding the sequence and clearing receipts. Delete or expiry removes stored
fixtures and receipts. Already-admitted work retains its selected response;
late completion cannot recreate removed or reset diagnostic state.

## Transport faults

An optional `fault` on a step deliberately prevents a complete HTTP response.
Faults are supported for **HTTP/1.1** through SleepAPI's instrumented listener.
Unsupported request versions or a router served without that listener reject
the attempt before it consumes a sequence position. Normal simulation and fixture
requests do not require transport instrumentation.

Library integrations must serve `router(config)` using `FaultListener::new(listener)`
and `into_make_service_with_connect_info::<FaultConnection>()`, as the bundled
binary does. Manually constructing connection metadata without the instrumented
socket does not support faults.

```json
{
  "sequence": [
    {"status_code":503,"duration_ms":20,"fault":{"type":"connection_drop"}},
    {"status_code":200,"duration_ms":20,"fixture":{"body":"abcdef","content_type":"text/plain"},"fault":{"type":"truncated_response","after_bytes":3}},
    {"status_code":200,"duration_ms":0,"fixture":{"body":"recovered","content_type":"text/plain"}}
  ]
}
```

- `connection_drop` aborts the connection before the final response headers.
  The operating system may signal EOF or reset. A request using
  `Expect: 100-continue` can receive an interim `100 Continue`.
- `truncated_response` requires a fixture and an `after_bytes` value greater than
  zero and smaller than the fixture's UTF-8 byte length. The server declares the
  full body length, writes exactly that prefix, then aborts before the remaining
  bytes. Cutoffs count bytes and may split a Unicode character. The client must
  observe an incomplete response; the connection cannot be reused.

The configured delay occurs before the fault. Each admitted attempt reserves
one step and retains its shared capacity slot through injection or cancellation.
Clients initiate their own retries; exhaustion and generation guards behave as
on ordinary recovery runs. Reset, deletion, and expiry do not cancel admitted
work, and its late diagnostics cannot reappear in cleared state.

Fault receipts retain the selected fault and its injection stage separately from
the outcome. An injected outcome records the server's action; it does not prove
what the client received. Cancellation before injection remains cancellation.
The receipt's `fault` contains the selected configuration. Only after injection,
`outcome` becomes `fault_injected`, `injection_stage` is `before_headers` or
`after_body_prefix`, and `injected_body_bytes` is zero or the selected cutoff.
The byte count describes the prefix accepted by the server socket, not confirmed
client delivery. A body abandoned before injection records `handler_cancelled`.
Always fetch receipts through a separate request; a deliberately broken response
cannot reliably carry diagnostics. Storage and lifetime use the same bounded
run/receipt limits above, and the process shutdown deadline still applies.

### Simulate acceptance with a lost response

Use `accept_then_drop` to test how a worker reconciles an ambiguous transport
failure after simulated provider acceptance. It requires a supported `2xx`
`status_code`. Discover support with `GET /metadata` and look for
`capabilities.accept_then_drop: true` on the HTTP/1.1 path used by the worker.
The capability check in this checkout accepts `--capability accept_then_drop`.
Published alpha.5 omits this capability. Check the running server's metadata
even when two builds declare the same package version.

Send this configuration to `POST /runs` and call the attempts endpoint twice
with the returned generation:

```json
{
  "sequence": [
    {"status_code":200,"duration_ms":0,"fault":{"type":"connection_drop"}},
    {"status_code":200,"duration_ms":0,"fault":{"type":"accept_then_drop"}}
  ]
}
```

Both faults drop the final HTTP response. A direct HTTP/1.1 request without
`Expect: 100-continue` receives zero response bytes. When the client requests
it, an interim `100 Continue` may precede step selection.

Both injected faults produce a receipt with `fault_injected`, `before_headers`,
and `injected_body_bytes: 0`. On a build supporting `accept_then_drop`, the
existing `connection_drop` fault explicitly records `provider_accepted: false`.
The new fault records `provider_accepted: true` at injection, even when both
steps select the same `200` status. Published alpha.5 omits the field on an
injected `connection_drop`. An absent field or a missing receipt leaves
acceptance unknown.

Pending or cancelled attempts, ordinary responses, and truncations omit the
field. The acceptance record exists only in the simulator. Read it from the
same SleepAPI process while the generation is current and the receipt is among
the 32 most recent attempts. Reset, expiry, deletion, or a server restart can
clear it.

See the [synthetic worker comparison](release-evidence/2026-11-23-lost-reply/README.md)
for raw byte counts, receipts, restart decisions, and commands to repeat the test.

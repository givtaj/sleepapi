# sleepapi

An Axum + Tokio API for simulating slow HTTP responses. Choose how long a request
waits, how it waits, and which HTTP status comes back. Use it to exercise loading
states, client timeouts, retries, and mock email or SMS integrations.

## Run with Docker

The image runs Linux on `amd64` and `arm64`. Use Docker Engine on Linux or Docker
Desktop in Linux-container mode on macOS and Windows; Rust is not needed on the
host. Build locally for your machine's architecture:

```sh
git clone https://github.com/givtaj/sleepapi.git
cd sleepapi
docker build -t sleepapi:0.1.0 .
docker run --rm --name sleepapi -p 127.0.0.1:3000:3000 sleepapi:0.1.0
```

The container runs as a non-root user and listens on `0.0.0.0:3000` internally.
The command above exposes it only on your host's loopback address. Docker selects
the matching architecture when building; the same Dockerfile serves both Intel/AMD
and ARM64 hosts. This repository currently provides a buildable image definition,
not a published registry image.

Configure the container with environment variables, for example:

```sh
docker run --rm --name sleepapi -p 127.0.0.1:3000:3000 -e SLEEPAPI_CORS_ORIGINS=http://localhost:5173 sleepapi:0.1.0
```

Stop it from another terminal with `docker stop --time 7 sleepapi`. The default
application grace period is five seconds; keep Docker's stop timeout longer than
`SLEEPAPI_SHUTDOWN_GRACE_MS` when overriding that setting. Docker forwards SIGTERM
to the binary running as PID 1.

## Run with Rust

Install Rust 1.94 or newer, then from this directory:

```sh
cargo run
```

The server listens at `http://127.0.0.1:3000`. In another terminal:

```sh
# Wait 400 ms without blocking a runtime worker.
curl -sS http://127.0.0.1:3000/sleep/tokio \
  -H 'Content-Type: application/json' \
  -d '{"duration_ms":400}'

# Simulate an email provider taking 1.2 seconds to reply.
curl -sS http://127.0.0.1:3000/simulate/email \
  -H 'Content-Type: application/json' \
  -d '{"duration_ms":1200,"method":"tokio_spawn"}'

# Simulate an SMS provider returning a delayed failure.
curl -i http://127.0.0.1:3000/simulate/sms \
  -H 'Content-Type: application/json' \
  -d '{"scenario":"delayed_failure","method":"thread_park"}'
```

Example result (elapsed time varies):

```json
{
  "operation": "sleep",
  "simulated": true,
  "method": "tokio_sleep",
  "requested_duration_ms": 400,
  "elapsed_ms": 401.2,
  "status_code": 200,
  "blocks_thread": false
}
```

Email and SMS are operation labels on the same delay engine. Nothing is sent;
no recipient, credentials, or external provider is involved. The HTTP connection
stays open until the delay completes, including when selecting `tokio_spawn` or
a status such as `202`. This version has no background-job queue or polling API.

## Waiting methods

| Endpoint | `method` | What waits | Implementation |
| --- | --- | --- | --- |
| `POST /sleep/tokio` | `tokio_sleep` (default) | The request's async task | Await `tokio::time::sleep` directly |
| `POST /sleep/tokio-spawn` | `tokio_spawn` | A separate async task; the handler awaits its result | `tokio::spawn` + timer + await the join handle |
| `POST /sleep/thread` | `thread_sleep` | An OS thread in Tokio's blocking pool | `spawn_blocking` + `std::thread::sleep` |
| `POST /sleep/park` | `thread_park` | An OS thread in Tokio's blocking pool | `spawn_blocking` + repeated `std::thread::park_timeout` |

An async task is not an OS thread. A Tokio timer suspends the task while runtime
workers can do other work. Thread sleeping and parking occupy a blocking-pool
thread for the duration; they do not run on the async worker threads.
`/sleep/tokio` awaits the timer inside the request task. `/sleep/tokio-spawn`
creates a separate Tokio task to await the timer, then the request task awaits
that task's join handle. Both keep the HTTP request open until the timer finishes;
spawning adds task scheduling and joining without making the timer faster.

The original inspiration was a thread that sleeps, stores a result, then calls
`waker.wake()`. Here, Tokio's timer or join handle provides that wake-up mechanism.
There is no hand-written `Future` or manual waker in this first version.

Parking differs from sleeping because it can be interrupted by an unpark token
or a spurious wake. The parking implementation checks elapsed time and parks
again until the requested duration is complete. This is a timed parking experiment;
there is no HTTP unpark endpoint. See the [Rust parking documentation](https://doc.rust-lang.org/std/thread/fn.park_timeout.html).

## Scenarios

Choose a scenario independently of how the server waits. All seven presets work
on all four named routes, and on `/sleep`, `/simulate/email`, and `/simulate/sms`.
`GET /scenarios` lists the presets and their defaults.

| `scenario` | Default delay | HTTP status | Client behavior to exercise |
| --- | --- | --- | --- |
| `fast_success` | 50 ms | 200 | Normal successful response |
| `slow_provider` | 2000 ms | 200 | Loading state during a slow dependency |
| `delayed_failure` | 800 ms | 503 | Temporary failure and retry/backoff handling |
| `client_timeout` | 3000 ms | 200 | Client abandons the request before a response arrives |
| `rate_limited` | 100 ms | 429 | Client respects the retry delay |
| `invalid_request` | 50 ms | 400 | Client displays an error and fixes input |
| `upstream_timeout` | 2000 ms | 504 | Client receives a gateway timeout response |

```sh
curl -i http://127.0.0.1:3000/sleep/tokio-spawn \
  -H 'Content-Type: application/json' \
  -d '{"scenario":"delayed_failure"}'

# Use the same scenario with a different waiting mechanism and shorter delay.
curl -i http://127.0.0.1:3000/sleep/thread \
  -H 'Content-Type: application/json' \
  -d '{"scenario":"rate_limited","duration_ms":50}'

# Expected: curl times out with exit code 28, before the server's 200 response.
curl --max-time 0.5 http://127.0.0.1:3000/sleep/park \
  -H 'Content-Type: application/json' \
  -d '{"scenario":"client_timeout"}'
```

`duration_ms` overrides a preset's delay, including `0`. A preset fixes the
status: sending both `scenario` and `status_code` returns `422`. Omit `scenario`
to choose an arbitrary supported `status_code`. Configured duration and capacity
limits apply to presets too. `scenario`, `duration_ms`, and `status_code` may be
omitted or set to `null`; a provided `method` must be a valid method name.

`client_timeout` does not manufacture a server error: your client's deadline
causes the timeout. `upstream_timeout` produces an actual HTTP `504` response.
Presets are stateless, so repeated failure requests keep failing; there is no
"fail twice, then succeed" counter or automatic server-side retry.

Concurrent bursts, overload, and cancellation are client-driven experiments:
send parallel requests to a named route, lower `SLEEPAPI_MAX_IN_FLIGHT` to reach
capacity, or cancel a pending request. The tests check concurrent timers, health
responsiveness on a single runtime thread, cancellation cleanup, continued
started blocking work, and parking through early wake-ups. The thread and park
routes really occupy blocking-pool threads; these are not aliases for a Tokio timer.

## Failure responses

Every simulated `4xx` or `5xx` response includes timing metadata and a structured
`error` with a stable `code`, readable `message`, and `retryable` hint. For example,
`delayed_failure` returns HTTP `503`, `Retry-After: 1`, and a body like this:

```json
{
  "operation": "sleep",
  "simulated": true,
  "method": "tokio_spawn",
  "scenario": "delayed_failure",
  "requested_duration_ms": 800,
  "elapsed_ms": 801.2,
  "status_code": 503,
  "blocks_thread": false,
  "error": {
    "code": "service_unavailable",
    "message": "Simulated provider is temporarily unavailable. Retry later.",
    "retryable": true
  }
}
```

The failure presets use `invalid_request` (400, not retryable), `rate_limited`
(429, retryable), `service_unavailable` (503, retryable), and `upstream_timeout`
(504, retryable). Custom `408`, `500`, and `502` responses are also marked
retryable; other custom error statuses are not. Clients decide whether and how
to retry. For `429` and `503`, `Retry-After: 1` suggests waiting one second; this
does not promise the next attempt succeeds. See the HTTP specifications for
[429](https://www.rfc-editor.org/rfc/rfc6585.html#section-4) and
[Retry-After](https://www.rfc-editor.org/rfc/rfc9110.html#name-retry-after).

Custom `401` responses include a representative `WWW-Authenticate: Bearer
realm="sleepapi"` challenge. Custom `405` responses include `Allow: GET`, modeling
a provider that expects GET. These are mock provider responses; the simulation
request is still POST, and no credentials are checked. Actual routing errors
report the real allowed methods (for example, `Allow: POST` on a sleep route).

This API does not model range responses, delta encoding, proxy authentication,
or protocol upgrades. Status codes `206`, `226`, `407`, `416`, and `426` are
rejected with `422`, as are bodyless statuses `204`, `205`, and `304`. Other custom
statuses simulate the status and JSON result, not a complete provider protocol;
for example, a `302` response does not configure a redirect destination.

Actual API rejections are immediate and use `simulated: false`. For example,
exhausting the shared capacity returns HTTP `429`, `Retry-After: 1`, and:

```json
{
  "simulated": false,
  "status_code": 429,
  "error": {
    "code": "capacity_exceeded",
    "message": "All simulation slots are busy; retry later.",
    "retryable": true
  }
}
```

## HTTP contract

| Endpoint | Behavior |
| --- | --- |
| `GET /health` | Immediate health response, independent of simulation capacity |
| `GET /methods` | Methods, defaults, and configured limits |
| `GET /scenarios` | Available presets, descriptions, default delays and statuses |
| `POST /sleep/tokio` | Wait with a Tokio timer in the request task |
| `POST /sleep/tokio-spawn` | Wait with a Tokio timer in a separate task, then join it |
| `POST /sleep/thread` | Sleep a thread in the blocking pool |
| `POST /sleep/park` | Park a thread in the blocking pool |
| `POST /sleep` | Select a waiting method in the JSON body |
| `POST /simulate/email` | Same wait, labeled as a simulated email operation |
| `POST /simulate/sms` | Same wait, labeled as a simulated SMS operation |

All POST endpoints accept `scenario`, `duration_ms`, and `status_code`. Send `{}` to use the
defaults. The four named `/sleep/...` endpoints fix the waiting method in the
route and reject a `method` field in the body, even if it matches the route.

The generic `/sleep`, `/simulate/email`, and `/simulate/sms` endpoints remain
available and also accept `method` to choose how to wait.

| Field | Default | Accepted values |
| --- | --- | --- |
| `duration_ms` | `400` | Integer from `0` through the configured maximum |
| `method` | `"tokio_sleep"` | One of the four names above; generic endpoints only |
| `status_code` | `200` | Integer `200`–`599`, except `204`, `205`, `206`, `226`, `304`, `407`, `416`, and `426` |
| `scenario` | None | One of the seven preset names above; selects default delay and status |

Bodyless and unsupported protocol-specific statuses are excluded as described above. The requested
status is the actual HTTP response status, including simulated errors. Unknown
fields are rejected to catch configuration typos. Bodies are limited to 8 KiB.

Malformed JSON returns `400`; invalid fields, excessive duration, or disallowed
status codes return `422`; missing JSON content type returns `415`; oversized
bodies return `413`; unknown operations return `404`. These errors have the shape
shown for actual API rejections above. Unsupported HTTP methods return `405` in
the same format. Requests arriving when all simulation slots are occupied get
an immediate `429`, without entering a waiting queue. Simulated failures have
the normal result shape with `simulated: true` plus the structured `error` object.

`elapsed_ms` measures server time from admission to wait completion, including
task scheduling and blocking-pool queue time. It excludes body parsing, response
serialization, and network travel. Delays are approximate and can overshoot due
to scheduling and timer resolution; this is not a precise benchmarking clock.

## Browser clients

Allow your frontend's exact origin to enable cross-origin JSON requests:

```sh
SLEEPAPI_CORS_ORIGINS=http://localhost:5173 cargo run
```

From that frontend:

```js
const response = await fetch("http://127.0.0.1:3000/sleep/tokio", {
  method: "POST",
  headers: { "Content-Type": "application/json" },
  body: JSON.stringify({ scenario: "rate_limited" }),
});
console.log(response.status, response.headers.get("Retry-After"));
console.log(await response.json());
```

Use a comma-separated list for multiple origins, such as
`http://localhost:5173,http://127.0.0.1:5173`. Origins contain a scheme, host, and
optional port, with no trailing slash or path. Wildcards and credential-bearing
URLs are rejected. An unset or empty list leaves cross-origin access disabled.
This permits GET, HEAD, and POST with `Content-Type` and `Authorization` headers,
and exposes `Retry-After`, `WWW-Authenticate`, and `Allow` to JavaScript. Cookie
credentials are not enabled. CORS controls browser access; it is not authentication.

## Configuration

| Environment variable | Default | Meaning |
| --- | --- | --- |
| `SLEEPAPI_ADDR` | `127.0.0.1:3000` | Listen IP and port (IPv6 uses `[::1]:3000`) |
| `SLEEPAPI_MAX_DURATION_MS` | `30000` | Maximum requested wait; `1`–`3600000` |
| `SLEEPAPI_MAX_IN_FLIGHT` | `64` | Shared active-wait limit; `1`–`1024` |
| `SLEEPAPI_SHUTDOWN_GRACE_MS` | `5000` | Time to drain after first Ctrl-C/SIGTERM; `1`–`60000` |
| `SLEEPAPI_CORS_ORIGINS` | Empty | Comma-separated exact frontend origins; empty disables CORS |
| `RUST_LOG` | `sleepapi=info` | Tracing filter |

```sh
SLEEPAPI_ADDR=127.0.0.1:3100 \
SLEEPAPI_MAX_DURATION_MS=60000 \
SLEEPAPI_MAX_IN_FLIGHT=128 \
cargo run
```

If the configured maximum is below the default 400 ms, provide `duration_ms`
explicitly. Invalid configuration stops startup. The server defaults to loopback;
it is a local development tool with no authentication.

All POST routes share the same capacity pool. The limit counts waiting work,
including blocking work that outlives its request future. It does not cap all TCP
connections or all HTTP requests. Tokio's blocking pool has its own thread limit;
work can queue there if your configured admission limit is larger.

When a request future is dropped, direct async sleeps are dropped and spawned
async sleeps are aborted. A client disconnect does not guarantee immediate
request-future cancellation. Already-started blocking work runs to completion
and keeps its capacity slot until it finishes during normal operation.

Ctrl-C or SIGTERM stops accepting connections and starts the configured shutdown
grace period. The server exits with code `0` when requests and started blocking
work finish. A second signal or the grace deadline forces process exit with code
`2`, including when a client never finishes its request body or blocking work is
still running. Choose a longer grace period if your test needs pending requests
to complete. The deadline terminates the process; it does not make OS-thread
sleep or parking individually cancellable.
See [Tokio's blocking-task lifecycle](https://docs.rs/tokio/latest/tokio/task/fn.spawn_blocking.html).

## Develop

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

GitHub Actions runs only on Linux. It checks formatting, Clippy, tests, release
builds, and source packaging on stable Rust, and tests the supported minimum Rust
1.94.0. Separate native Linux `amd64` and `arm64` jobs build the Docker image and
verify container HTTP behavior, CORS, and shutdown. No images are pushed by CI.
Process tests cover graceful drain, stalled request bodies, repeated signals,
and blocking work at shutdown.

Run the container checks locally with Docker and Python 3:

```sh
docker build -t sleepapi:ci .
python3 scripts/docker-smoke.py sleepapi:ci
```

The code is split into the HTTP contract (`src/lib.rs`), the four waiting
implementations (`src/wait.rs`), environment configuration (`src/config.rs`),
scenario presets (`src/scenario.rs`), error responses (`src/error.rs`),
and server startup/shutdown (`src/main.rs`).

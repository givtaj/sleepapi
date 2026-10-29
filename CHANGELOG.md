# Changelog

## 0.1.1-alpha.4 — 2026-10-29

### Added

- Exact bounded UTF-8 response fixtures on recovery-run steps, with a content-type
  and header allowlist. Simulation diagnostics remain in separate run receipts.
- Explicit HTTP/1.1 connection-drop and truncated-response faults, with bounded
  injection receipts, shared capacity, and existing run lifecycle guarantees.
- Fixture contract and real TCP fault regressions, plus container smoke coverage.
  See the [fixture and fault API](docs/response-fixtures.md).

## 0.1.1-alpha.3 — 2026-10-28

### Added

- Isolated in-memory recovery runs with finite response sequences, generation
  guards, deterministic assignment at admission, explicit reset/deletion, and
  automatic expiry. Runs share the existing simulation capacity and waiting methods.
- Bounded run, step, lifetime-attempt and receipt storage, with diagnostics that
  cannot be repopulated by late completions after reset, deletion or expiry.
- Recovery-run contract tests and Docker HTTP coverage. See the
  [run API](docs/recovery-runs.md) for limits and cancellation semantics.

## 0.1.1-alpha.2 — 2026-10-28

### Fixed

- Keep HTTP requests and bounded shutdown responsive when stdout stops being
  drained. Use a bounded, lossy logging queue and keep logger teardown under
  the supervisor's grace deadline and second-signal escape.
- Add subprocess regressions with a deliberately saturated stdout pipe for
  request progress, logging teardown, grace expiry, and repeated signals.

### Documentation

- Clarify the historical benchmark inventory, source limits, and requirements
  for reproducing the recorded runs. No new throughput benchmark is claimed.

HTTP behavior is unchanged. Logging uses a bounded, lossy queue; records may be
dropped under pressure and final shutdown logs are best effort. Logger teardown
shares the shutdown deadline, so blocked output can cause exit `2` even after
HTTP work drains. See the [release verification record](https://github.com/givtaj/sleepapi/releases/tag/v0.1.1-alpha.2).

## 0.1.1-alpha.1 — 2026-10-26

### Fixed

- Prevent a resource-sampler shutdown deadlock when SIGINT or SIGTERM arrives
  during a timed wait; check stop-file requests promptly at long sample intervals.

### Added

- Sampler shutdown regressions and a repeatable real-HTTP alpha acceptance gate
  with response, latency, health, and capacity-recovery thresholds in CI.
- An alpha readiness record and version-independent release procedure that
  preserve the already-published v0.1.0 release.

The API and source-plus-Dockerfile distribution remain the same. This alpha
does not add registry images, native downloads, hosted service, or Windows
verification. See the [release verification record](https://github.com/givtaj/sleepapi/releases/tag/v0.1.1-alpha.1)
and [acceptance gates](docs/ALPHA_READINESS.md).

## 0.1.0 — 2026-09-30

Initial developer-tool release for simulating HTTP latency and provider failures.

### Added

- Four waiting endpoints: direct Tokio timer, spawned Tokio timer, blocking thread
  sleep, and timed thread parking.
- Generic sleep requests and mock email/SMS operations with configurable delays
  and supported HTTP response statuses.
- Seven scenario presets, including slow providers, client timeouts, rate limits,
  delayed failures, and upstream timeouts.
- Structured simulated errors and API rejections, retry hints, and representative
  authentication and method headers.
- Method/scenario discovery, health checks, request validation, body limits, and
  shared simulation capacity.
- Configurable browser CORS and bounded shutdown on Ctrl-C/SIGTERM.
- A Dockerfile for building a non-root Linux container on AMD64 and ARM64.
- Linux CI covering Rust stable and 1.94.0, native builds for both container
  architectures, and container HTTP/shutdown checks.

### Initial limits

- Email and SMS are labels; no messages are sent and no provider is contacted.
- Requests remain open until completion; there is no background queue or polling API.
- Presets are stateless. Timing is approximate, and cancellation cannot stop an
  already-started blocking thread; the process shutdown deadline still applies.
- This is a local development service without authentication. Docker examples
  expose only the host loopback interface.
- Distribution is source code plus the Dockerfile. Prebuilt registry images,
  native binary downloads, and crates.io publication are outside v0.1.0's scope.
- CI covers Linux AMD64 and ARM64. Docker Desktop was checked on macOS; Windows
  host integration and native Windows builds have not been tested.

Publication and verification are tracked in [the release checklist](docs/RELEASING.md).

# Changelog

## 0.1.1-alpha.5 — 2026-11-10 (planned)

The planned source-prerelease date is **10 November 2026 (Europe/Madrid)**.
Publication remains pending; this dated entry does not announce a release.
The public quickstart builds alpha.4 from source until alpha.5 is published and
its tagged-source installation is verified. No alpha.5 tag or release has been
published, and no Docker images will be published.

The [clean-source verification](docs/release-evidence/2026-11-10-alpha5/README.md)
passed on the recorded candidate. The final documentation commit still requires
its own green main CI before any separately authorized tag or source release.

### Added

- A read-only `GET /metadata` endpoint reporting versioned capability metadata
  and configured limits, plus a loopback capability-check client with explicit
  supported, unsupported, and unknown results. This is an unreleased source
  addition; the published alpha.4 tag and image remain unchanged.
- A runnable Python retry client with bounded retries, exact fixture validation,
  client-observed transport failures, separate receipts, and reset/replay checks.
- Real TCP disconnect regressions for ordinary recovery attempts, covering
  sequence consumption, cancellation/capacity, and reset/deletion cleanup.

### Distribution and onboarding

- Distribute source plus the Dockerfile. The quickstart builds the pinned source
  locally with Docker's configured platform defaults; CI still tests native
  Linux AMD64 and ARM64 images without publishing them.
- Remove the registry-image publication workflow, its helper, and the eight
  publication-only regression tests. Existing alpha.4 images and their release
  evidence remain historical artifacts; new images will not be published.
- A contribution guide and focused bug-report and feature-request templates.

### Documentation

- Correct recovery-run response and receipt descriptions for alpha.4 fixtures
  and transport faults, and document consumer acceptance of the pinned release.
- Add the reproducible Linux ARM64 benchmark and its retained verification
  evidence; its measurements describe the recorded source and workload.

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

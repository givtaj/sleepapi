# Changelog

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

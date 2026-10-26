# SleepAPI alpha readiness

Assessment date: 2026-10-26 (Europe/Madrid). Prepared version:
**`0.1.1-alpha.1`**. Publication status: **not released**.

## Where the project stands

SleepAPI already has a public [v0.1.0 source release](https://github.com/givtaj/sleepapi/releases/tag/v0.1.0),
published on 2026-09-30 in the local timezone. Its annotated tag resolves to
`d6e10c3012b042e825831606424c41437e20a06c`. The starting `main` for this assessment
was `6f282c5b2147240d8996bc879898d7d92ee042ff`, matching GitHub, with
[successful CI](https://github.com/givtaj/sleepapi/actions/runs/37206574616).
Those results are historical and do not validate this new candidate.

The API is feature-complete for a small local-development alpha: four real
waiting implementations, seven scenario presets, mock email/SMS labels, validated
JSON, browser CORS, admission limits, and bounded process shutdown. Review found
no runtime defect blocking this scope. The remaining work is candidate
verification and release publication, rather than a new feature milestone.

The audit reproduced a resource-sampler deadlock when a signal interrupted its
timed wait. This candidate fixes it, makes stop-file checks prompt at long sample
intervals, and adds regression coverage. It carries forward the functional fix
from the conflicting [migration PR #1](https://github.com/givtaj/sleepapi/pull/1)
without importing duplicate historical evidence. That PR remains separate
repository housekeeping; it must not be merged wholesale into this candidate.

## Alpha scope

- Local developer tool, source plus Dockerfile. Linux AMD64 and ARM64 are the
  container targets; macOS native execution is a local verification environment.
- Existing HTTP behavior and defaults: 30-second maximum requested wait,
  64 active-wait slots, 8 KiB request body, five-second shutdown grace.
- No hosted-service, production capacity, prebuilt image/binary, provider delivery,
  background-job, authentication, or Windows verification claim.

The 64-slot default is an admission limit, not a load-tested capacity guarantee.
The larger configurable limits remain experimental. The
[October 4 benchmarks](benchmark-results/2026-10-04-followup/README.md) provide
useful historical evidence (13,260 successful measured responses and 60 contract
checks) on their recorded source and host, not fresh candidate certification.

## Release thresholds

Every required gate must pass. A skipped, missing, or stale result is pending,
not green. Fixes create a new candidate; do not relax a threshold to pass a run.

| Gate | Required threshold | Evidence |
| --- | --- | --- |
| Defects | Zero unresolved critical/high-severity defects in supported behavior: crashes, hangs, incorrect HTTP contracts, or isolation failures | Review of final diff and failure triage; this is not a claim of a security audit |
| Rust quality and contract | Formatting clean, zero Clippy warnings, all tests pass with zero ignored failures on stable and Rust 1.94.0; release builds succeed | Both Linux Rust CI jobs; current suite has 39 tests |
| Wait and lifecycle correctness | No early response; capacity returns structured `429` when full and recovers; dropped async work releases capacity; started blocking work retains it; graceful exit `0`, deadline/second-signal exit `2` | Rust API/wait/process tests and Docker smoke; process tests use a 3-second observation timeout, not a 150-ms timing promise |
| Sampler shutdown | Both SIGINT and SIGTERM and a stop file finish within 1 second in the long-interval fixture; preserve samples and restore handlers on errors | Six Python regression tests; polling interval is at most 100 ms, excluding setup/I/O |
| Local HTTP regression | 64 requests on each of 4 routes at concurrency 16 and 40-ms delay: 256 valid responses, zero unexpected statuses/timeouts/transport errors; per-route p95 client overhead ≤250 ms; every health probe ≤250 ms | `scripts/alpha-smoke.py`; two Tokio workers, loopback, two-second request timeout; nearest-rank p95 includes HTTP client/network overhead |
| Admission recovery | For every method, prove capacity-one rejection during a 300-ms wait, health stays ≤250 ms while occupied, and a real simulation succeeds within 1,300 ms of the admitted attempt’s launch | Separate process in alpha smoke; tests ordinary completion, while Rust tests cover cancellation semantics |
| Distribution | All four CI jobs pass for the final release commit; clean package builds; both native Linux container architectures pass HTTP/CORS/non-root/shutdown smoke | `ci.yml`, followed by an annotated tag on that same commit and fresh public tagged-source installation |

The numeric latency limits are deliberately broad regression guards for the
fixed smoke workload on the recorded host. They are not service SLOs or maximum
throughput measurements. CPU/RSS/thread counts remain descriptive until a fresh,
controlled benchmark establishes comparable budgets; the alpha does not invent
hardware-independent resource thresholds from one old run.

## Candidate evidence

Local verification below was recorded before committing the preparation branch.
The preparation PR records its exact commit and current CI results. The final release
must link a successful `main` CI run on its actual tag target.

| Check | Current result |
| --- | --- |
| Stable format, Clippy, 39 Rust tests | Passed locally on macOS ARM64 |
| Rust 1.94.0 tests | 39 passed locally; minimum-toolchain Clippy is enforced by CI |
| Versioned release builds | Passed locally on stable and Rust 1.94.0 |
| Source package | Passed local `cargo package --locked --allow-dirty` snapshot verification; CI requires clean packaging |
| Python sampler regressions | 6 passed; original code reproduces signal deadlocks and slow stop-file handling |
| Alpha HTTP receipt | [Passed on macOS ARM64](release-evidence/2026-10-26/alpha-smoke-macos-arm64.json): 256/256 valid; worst p95 overhead 24.699 ms, health 15.747 ms, recovery 307.761 ms |
| Final candidate CI, both Docker architectures | Check the preparation PR’s checks and description for current results; local Docker daemon was stopped during this audit |
| Tag, fresh public tagged installation, prerelease | Pending publication |

## Remaining release sequence

1. Review and merge the prepared candidate after its PR checks pass. Set the
   actual changelog release date and update candidate/publication wording before
   the final commit; then require all four `main` CI jobs green on that commit.
2. Create immutable annotated tag `v0.1.1-alpha.1` at the tested commit. Follow
   [Releasing](RELEASING.md) to verify a fresh public tagged checkout and Docker
   build, HTTP/shutdown smoke, and the source archive.
3. Publish the GitHub **prerelease**, recording the full commit, CI URL, smoke
   receipt, installation result, and platform limitations. Verify signed-out
   access before announcing it.

Preparation does not create a tag, publish a release, or validate a hosted
deployment. The existing `v0.1.0` remains available throughout this process.

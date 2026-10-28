# SleepAPI alpha readiness

Assessment date: 2026-10-28 (Europe/Madrid). Alpha version:
**`0.1.1-alpha.3`**. The [GitHub prerelease record](https://github.com/givtaj/sleepapi/releases/tag/v0.1.1-alpha.3)
is the authoritative publication status and completion evidence, including the
released commit, final CI, and fresh tagged-source installation.

This candidate adds [recovery runs](recovery-runs.md): isolated, finite response
sequences with generation guards, reset, deletion, expiry, and bounded diagnostic
receipts. Final candidate CI and tagged-source verification are required before
publication; the linked release record captures their results.

## Where the project stands

The previous [v0.1.1-alpha.1 source release](https://github.com/givtaj/sleepapi/releases/tag/v0.1.1-alpha.1)
was published on 2026-10-26. It passed its original 39-test suite, but a later
report exposed a coverage gap: an undrained stdout pipe could stall HTTP requests
and prevent the shutdown deadline and repeated signals from terminating the process.

The previous [alpha.2 release](https://github.com/givtaj/sleepapi/releases/tag/v0.1.1-alpha.2)
includes the [issue #3 fix](https://github.com/givtaj/sleepapi/pull/7):
a bounded, lossy logging queue keeps blocking output off request workers and the
shutdown supervisor. Logger teardown stays within the supervisor's grace period.
Three process tests cover request progress, grace expiry, and repeated signals
with stdout deliberately saturated. Logging can drop records under pressure;
final shutdown messages are best effort, and blocked logger teardown can cause
exit `2` even when HTTP work has drained.

The [historical benchmark documentation](https://github.com/givtaj/sleepapi/pull/1)
has also been reconciled and merged. It describes the retained evidence and rerun
limits; it does not certify this candidate's throughput. Recovery sequences
([#4](https://github.com/givtaj/sleepapi/issues/4)) are the addition in alpha.3. Exact response fixtures ([#5](https://github.com/givtaj/sleepapi/issues/5))
and transport faults ([#6](https://github.com/givtaj/sleepapi/issues/6)) remain
future work.

## Alpha scope

- Local developer tool, source plus Dockerfile. Linux AMD64 and ARM64 are the
  container targets; macOS native execution is a local verification environment.
- Existing HTTP behavior and defaults: four waiting implementations, seven
  scenarios, mock email/SMS labels, 30-second maximum requested wait, 64
  active-wait slots, 8 KiB request body, and five-second shutdown grace.
- Recovery runs: up to 64 live runs, 64 steps per run, 1,024 lifetime attempts per
  run, 32 retained receipts per run, and a fixed TTL of 1 ms to 1 hour (default
  five minutes). Attempts share simulation capacity; management does not consume
  it. State is process-local and disappears on restart.
- No hosted-service, production capacity, prebuilt image/binary, provider delivery,
  background-job, authentication, or Windows verification claim.

The 64-slot default is an admission limit, not a load-tested capacity guarantee.
Larger configurable limits remain experimental. The
[October 4 benchmarks](benchmark-results/2026-10-04-followup/README.md) are
historical evidence on their recorded source and host, not candidate certification.

## Release thresholds

Every required gate must pass. A skipped, missing, or stale result is pending,
not green. Fixes create a new candidate; do not relax a threshold to pass a run.

| Gate | Required threshold | Evidence |
| --- | --- | --- |
| Defects | Zero unresolved critical/high-severity defects in supported behavior: crashes, hangs, incorrect HTTP contracts, or isolation failures | Review of final diff and failure triage; this is not a claim of a security audit |
| Rust quality and contract | Formatting clean, zero Clippy warnings, all tests pass with zero ignored failures on stable and Rust 1.94.0; release builds succeed | Both Linux Rust CI jobs; 58 tests, including 16 recovery-run tests |
| Wait and lifecycle correctness | No early response; capacity returns structured `429` when full and recovers; dropped async work releases capacity; started blocking work retains it; graceful exit `0`, deadline/second-signal exit `2` | Rust API/wait/process tests and Docker smoke; existing process tests use a 3-second observation timeout |
| Recovery runs | Deterministic reservation order; rejected attempts consume no step; finite sequence exhaustion and stale generations reject; reset preserves lifetime budget and TTL; late completions cannot recreate cleared state | `tests/runs.rs` and recovery-inclusive Docker HTTP smoke |
| Blocked stdout | 2,048 requests progress with stdout saturated; logger teardown and incomplete-body shutdown terminate within 1 second with 150-ms grace; second signal terminates within 1 second during a 10-second grace | Three deterministic subprocess regressions; observation limits are test guards, not general latency promises |
| Sampler shutdown | Both SIGINT and SIGTERM and a stop file finish within 1 second in the long-interval fixture; preserve samples and restore handlers on errors | Six Python regression tests; polling interval is at most 100 ms, excluding setup/I/O |
| Local HTTP regression | 64 requests on each of 4 routes at concurrency 16 and 40-ms delay: 256 valid responses, zero unexpected statuses/timeouts/transport errors; per-route p95 client overhead ≤250 ms; every health probe ≤250 ms | `scripts/alpha-smoke.py`; two Tokio workers, loopback, two-second request timeout; nearest-rank p95 includes HTTP client/network overhead |
| Admission recovery | For every method, prove capacity-one rejection during a 300-ms wait, health stays ≤250 ms while occupied, and a real simulation succeeds within 1,300 ms of the admitted attempt's launch | Separate process in alpha smoke; tests ordinary completion, while Rust tests cover cancellation semantics |
| Distribution | All four CI jobs pass for the final release commit; clean package builds; both native Linux container architectures pass HTTP/CORS/non-root/shutdown smoke | `ci.yml`, followed by an annotated tag on that same commit and fresh public tagged-source installation |

The numeric latency limits are broad regression guards for the fixed smoke
workload on the recorded host. They are not service SLOs or maximum throughput
measurements. CPU/RSS/thread counts remain descriptive until a fresh, controlled
benchmark establishes comparable budgets.

## Candidate evidence

The merged implementation and documentation baseline,
`e325cc94a6b0bb3ca0d4c153a88a79c8dc88ff50`, passed
[all four CI jobs](https://github.com/givtaj/sleepapi/actions/runs/40708257954).
That evidence covers recovery runs and the completed release review before this
version and documentation update.
The final candidate must pass the same jobs on its own commit before tagging.

The [alpha.1 local receipt](release-evidence/2026-10-26/README.md) remains a
historical record. Current release CI, alpha smoke, and fresh-install results
belong in the linked alpha.3 prerelease notes, with their actual commit and date.

## Recovery-run release review — 2026-10-28

Reviewed the recovery-run change at
[`d5c296fdf448631948c10f00d0dbea438fb26720`](https://github.com/givtaj/sleepapi/commit/d5c296fdf448631948c10f00d0dbea438fb26720)
against published alpha.2. Independent lifecycle, HTTP-contract, and distribution
reviews found no release-blocking runtime or packaging defect. Two README
inaccuracies were corrected: DELETE is enabled by the configured CORS policy,
and run management does not consume simulation-capacity slots.

| Review area | Result and evidence |
| --- | --- |
| CI on the reviewed source | PASS: [main run 40706326174](https://github.com/givtaj/sleepapi/actions/runs/40706326174), stable Rust, Rust 1.94.0, native Linux AMD64 Docker and native Linux ARM64 Docker |
| Rust and Python regressions | PASS in that CI: 58 Rust tests per toolchain (including 16 recovery-run tests), six Python tests, lint, builds, HTTP acceptance and source-package verification |
| State lifecycle | PASS: bounded runs/steps/attempts/receipts, generation isolation, reservation ordering, cancellation, reset/delete/expiry and late-completion behavior reviewed |
| Real HTTP disconnects | PASS locally: TCP FIN and reset probes across all four wait methods; cancellation receipts, async capacity recovery, continued blocking-work capacity, and subsequent attempt positions checked |
| HTTP headers and browser access | PASS locally: simulated 401/405/429/503 headers, configured CORS, and actual routing 405 Allow headers |
| Distribution inventory | PASS: 80-file source package includes recovery source, tests, API documentation and Docker smoke; compiled build artifacts and desktop metadata are excluded |

The local disconnect probes used the existing Rust 1.94.0 release build from
issue #4 verification; its source files match the reviewed tree. These probes
supplement the automated tests, which cancel router futures directly; they are
not yet retained as wire-level regression tests. A cancellation receipt is not
proof that blocking work stopped, and completion is not proof of client delivery.
Client acceptance remains separate from this review.

The review recommended **`v0.1.1-alpha.3`** as a source-plus-Dockerfile
prerelease. This preparation follows that recommendation; the review itself
did not publish a release. Its CI results do not validate the later preparation
commit. Windows and production capacity remain unverified.

## Alpha.3 release completion sequence

1. Review and merge the prepared candidate after its PR checks pass. Set the
   actual changelog release date and update publication wording before the final
   commit; then require all four `main` CI jobs green on that commit.
2. Create immutable annotated tag `v0.1.1-alpha.3` at the tested commit. Follow
   [Releasing](RELEASING.md) to verify a fresh public tagged checkout and Docker
   build, HTTP/shutdown smoke, and the source archive.
3. Publish the GitHub **prerelease**, recording the full commit, CI URL, smoke
   receipt, installation result, and platform limitations. Verify signed-out
   access before announcing it.

The linked GitHub prerelease record captures the completed sequence without
changing the immutable release commit to insert its own SHA. A source document
alone does not prove publication or installation. Earlier releases remain
available; this release has no hosted deployment.

# SleepAPI alpha readiness

Assessment date: 2026-10-28 (Europe/Madrid). Alpha version:
**`0.1.1-alpha.2`**. The [GitHub prerelease record](https://github.com/givtaj/sleepapi/releases/tag/v0.1.1-alpha.2)
is the authoritative publication status and completion evidence, including the
released commit, final CI, and fresh tagged-source installation.

## Where the project stands

The previous [v0.1.1-alpha.1 source release](https://github.com/givtaj/sleepapi/releases/tag/v0.1.1-alpha.1)
was published on 2026-10-26. It passed its original 39-test suite, but a later
report exposed a coverage gap: an undrained stdout pipe could stall HTTP requests
and prevent the shutdown deadline and repeated signals from terminating the process.

This candidate includes the [issue #3 fix](https://github.com/givtaj/sleepapi/pull/7):
a bounded, lossy logging queue keeps blocking output off request workers and the
shutdown supervisor. Logger teardown stays within the supervisor's grace period.
Three new process tests cover request progress, grace expiry, and repeated signals
with stdout deliberately saturated. Logging can drop records under pressure;
final shutdown messages are best effort, and blocked logger teardown can cause
exit `2` even when HTTP work has drained.

The [historical benchmark documentation](https://github.com/givtaj/sleepapi/pull/1)
has also been reconciled and merged. It describes the retained evidence and rerun
limits; it does not certify this candidate's throughput. Optional recovery
sequences, response fixtures, and transport faults remain future work in issues
[#4](https://github.com/givtaj/sleepapi/issues/4),
[#5](https://github.com/givtaj/sleepapi/issues/5), and
[#6](https://github.com/givtaj/sleepapi/issues/6).

## Alpha scope

- Local developer tool, source plus Dockerfile. Linux AMD64 and ARM64 are the
  container targets; macOS native execution is a local verification environment.
- Existing HTTP behavior and defaults: four waiting implementations, seven
  scenarios, mock email/SMS labels, 30-second maximum requested wait, 64
  active-wait slots, 8 KiB request body, and five-second shutdown grace.
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
| Rust quality and contract | Formatting clean, zero Clippy warnings, all tests pass with zero ignored failures on stable and Rust 1.94.0; release builds succeed | Both Linux Rust CI jobs; current suite has 42 tests |
| Wait and lifecycle correctness | No early response; capacity returns structured `429` when full and recovers; dropped async work releases capacity; started blocking work retains it; graceful exit `0`, deadline/second-signal exit `2` | Rust API/wait/process tests and Docker smoke; existing process tests use a 3-second observation timeout |
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
`ea1737de24c777a660bf187dff128ad8ddb5c60d`, passed
[all four CI jobs](https://github.com/givtaj/sleepapi/actions/runs/40366257756).
That evidence covers the fix before this version and documentation update.
The final candidate must pass the same jobs on its own commit before tagging.

The [alpha.1 local receipt](release-evidence/2026-10-26/README.md) remains a
historical record. Current release CI, alpha smoke, and fresh-install results
belong in the linked alpha.2 prerelease notes, with their actual commit and date.

## Release completion sequence

1. Review and merge the prepared candidate after its PR checks pass. Set the
   actual changelog release date and update publication wording before the final
   commit; then require all four `main` CI jobs green on that commit.
2. Create immutable annotated tag `v0.1.1-alpha.2` at the tested commit. Follow
   [Releasing](RELEASING.md) to verify a fresh public tagged checkout and Docker
   build, HTTP/shutdown smoke, and the source archive.
3. Publish the GitHub **prerelease**, recording the full commit, CI URL, smoke
   receipt, installation result, and platform limitations. Verify signed-out
   access before announcing it.

The linked GitHub prerelease record captures the completed sequence without
changing the immutable release commit to insert its own SHA. A source document
alone does not prove publication or installation. Earlier releases remain
available; this release has no hosted deployment.

# SleepAPI alpha readiness

Assessment date: 2026-10-29 (Europe/Madrid). Alpha release candidate:
**`0.1.1-alpha.4`**. This candidate adds
[response fixtures and transport faults](response-fixtures.md) to recovery runs.
The implementation is merged; final candidate CI, tagging, fresh tagged-source
installation, and publication are separate release gates.

The [alpha.4 GitHub prerelease record](https://github.com/givtaj/sleepapi/releases/tag/v0.1.1-alpha.4)
is the destination for authoritative publication and completion evidence,
including the released commit, final CI, and installation results. This source
document does not establish that the candidate is published. The previous
[alpha.3 release](https://github.com/givtaj/sleepapi/releases/tag/v0.1.1-alpha.3)
remains available and verifies its own source only.

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
([#4](https://github.com/givtaj/sleepapi/issues/4)) shipped in alpha.3. Exact response
fixtures ([#5](https://github.com/givtaj/sleepapi/issues/5)) and transport faults
([#6](https://github.com/givtaj/sleepapi/issues/6)) are merged and included in the
alpha.4 candidate; see the [API contract](response-fixtures.md). Alpha.3's
historical release evidence below covers its original 58-test source only.

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
- Optional fixtures: exact decoded UTF-8 bodies up to 4,096 bytes, three allowed
  content types, and up to five allowed headers with 256-byte ASCII values. The
  complete run-creation request remains bounded by the existing 8 KiB limit.
- HTTP/1.1 faults through the instrumented listener: connection drop before
  headers or an exact fixture prefix followed by an incomplete response. Attempts
  retain capacity through injection or cancellation; diagnostics remain separate
  from the broken response and cannot restore cleared or evicted receipts.
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
| Rust quality and contract | Formatting clean, zero Clippy warnings, all tests pass with zero ignored failures on stable and Rust 1.94.0; release builds succeed | Both Linux Rust CI jobs; development source has 89 tests covering existing behavior, fixtures, transport faults, disconnects and lifecycle; released alpha.4 has 86 |
| Wait and lifecycle correctness | No early response; capacity returns structured `429` when full and recovers; dropped async work releases capacity; started blocking work retains it; graceful exit `0`, deadline/second-signal exit `2` | Rust API/wait/process tests and Docker smoke; existing process tests use a 3-second observation timeout |
| Recovery runs | Deterministic reservation order; rejected attempts consume no step; finite sequence exhaustion and stale generations reject; reset preserves lifetime budget and TTL; late completions cannot recreate cleared state | `tests/runs.rs` and recovery-inclusive Docker HTTP smoke |
| Fixtures and faults | Exact fixture bytes and allowed headers; no-header drop; byte-exact truncated prefix with incomplete declared length; rejected inputs consume no state; fault bodies retain capacity; late injection cannot restore cleared receipts | `tests/fixtures.rs`, `tests/transport.rs`, `tests/fault_lifecycle.rs`, transport unit tests and Docker client checks |
| Blocked stdout | 2,048 requests progress with stdout saturated; logger teardown and incomplete-body shutdown terminate within 1 second with 150-ms grace; second signal terminates within 1 second during a 10-second grace | Three deterministic subprocess regressions; observation limits are test guards, not general latency promises |
| Sampler shutdown | Both SIGINT and SIGTERM and a stop file finish within 1 second in the long-interval fixture; preserve samples and restore handlers on errors | Six Python regression tests; polling interval is at most 100 ms, excluding setup/I/O |
| Local HTTP regression | 64 requests on each of 4 routes at concurrency 16 and 40-ms delay: 256 valid responses, zero unexpected statuses/timeouts/transport errors; per-route p95 client overhead ≤250 ms; every health probe ≤250 ms | `scripts/alpha-smoke.py`; two Tokio workers, loopback, two-second request timeout; nearest-rank p95 includes HTTP client/network overhead |
| Admission recovery | For every method, prove capacity-one rejection during a 300-ms wait, health stays ≤250 ms while occupied, and a real simulation succeeds within 1,300 ms of the admitted attempt's launch | Separate process in alpha smoke; tests ordinary completion, while Rust tests cover cancellation semantics |
| Retry-client example | Both three-attempt sequences recover and repeat after reset; exact fixture bytes and observed faults match; stale generations consume no step; receipts correlate; owned runs are deleted | `examples/retry-client.py --binary target/release/sleepapi` in stable CI; synthetic Python-client evidence, separate from external client acceptance |
| Distribution | All four CI jobs pass for the final release commit; clean package builds; both native Linux container architectures pass HTTP/CORS/non-root/shutdown smoke | `ci.yml`, followed by an annotated tag on that same commit and fresh public tagged-source installation |

The numeric latency limits are broad regression guards for the fixed smoke
workload on the recorded host. They are not service SLOs or maximum throughput
measurements. CPU/RSS/thread counts remain descriptive until a fresh, controlled
benchmark establishes comparable budgets.

## Development coverage after alpha.4

The [retry-client example](client-acceptance.md) and five Python guard tests add
repeatable consumer observations without changing the server. The guard tests
reject misleading success from malformed framing, unexpected connection errors,
and incorrect fixture bytes. These additions are not part of the immutable
alpha.4 tag, although the example can exercise that release.

Three Unix TCP tests in `tests/disconnect.rs` retain the earlier manual
disconnect scenarios: all four waiting methods, full socket close and reset,
consumed sequence positions, capacity after cancellation, and reset/deletion
before disconnect. They cover 24 cases using the production listener. Blocking
cases prewarm the pool and allow 50 ms for worker startup before rechecking
pending state; the barrier-based wait unit test separately proves the semantics
of already-started blocking work. These tests do not promise immediate detection
of every client disconnect. Windows remains unverified.

## Alpha.4 implementation baseline — 2026-10-29

The merged fixture and transport-fault baseline,
[`535301ac7c4d2adcc6a8d33a6f121c71d5247d96`](https://github.com/givtaj/sleepapi/commit/535301ac7c4d2adcc6a8d33a6f121c71d5247d96),
passed [main CI run 40883400744](https://github.com/givtaj/sleepapi/actions/runs/40883400744):
Linux Rust stable, Rust 1.94.0, native Linux AMD64 Docker, and native Linux ARM64
Docker all succeeded. This baseline has 86 Rust tests and six Python regressions,
plus alpha HTTP acceptance and fixture/fault-aware Docker smoke checks.

Independent review of [implementation PR #12](https://github.com/givtaj/sleepapi/pull/12)
covered fixtures, runs, waiting, transport, server integration, tests, and docs,
with no actionable source findings. The review checked the flush handshake
against the locked Hyper version and cancellation/late-completion handling for
capacity and receipts; a pipelined truncation regression was added. This was not
external client acceptance or a security audit.

Automated coverage includes real TCP faults, exact response bytes, fault-body
cancellation and capacity, and reset/deletion/expiry/receipt-eviction isolation.
These results validate the implementation baseline. The later alpha.4
version and documentation commit must pass all four CI jobs itself before an
annotated release tag or publication. Fresh tagged-source installation remains a
separate gate; record its result in the alpha.4 prerelease notes.

## Alpha.3 preparation evidence (historical)

The merged implementation and documentation baseline,
`e325cc94a6b0bb3ca0d4c153a88a79c8dc88ff50`, passed
[all four CI jobs](https://github.com/givtaj/sleepapi/actions/runs/40708257954).
That evidence covered recovery runs and their completed release review before
the alpha.3 version and documentation update. It does not validate alpha.4.

The [alpha.1 local receipt](release-evidence/2026-10-26/README.md) remains a
historical record. Alpha.3's final CI, alpha smoke, and fresh-install results
belong to its [prerelease record](https://github.com/givtaj/sleepapi/releases/tag/v0.1.1-alpha.3),
with their actual commit and date.

## Recovery-run release review — 2026-10-28 (historical)

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

The local disconnect probes used the Rust 1.94.0 release build from
issue #4 verification; its source files matched the reviewed tree. Those probes
supplemented the router-future cancellation tests and were not retained as
wire-level regressions in alpha.3. Alpha.4 adds separate automated transport-fault
tests; it does not retroactively automate those historical probes. A cancellation
receipt is not proof that blocking work stopped, and completion is not proof of client delivery.
Client acceptance remains separate from this review.

The review recommended **`v0.1.1-alpha.3`** as a source-plus-Dockerfile
prerelease. The review itself did not publish that release, and its CI results
did not validate the later alpha.3 preparation commit. Windows and production
capacity remain unverified.

## Alpha.4 release completion sequence

1. Review and merge the prepared candidate after its PR checks pass. Set the
   actual changelog release date and update publication wording before the final
   commit; then require all four `main` CI jobs green on that commit.
2. Create immutable annotated tag `v0.1.1-alpha.4` at the tested commit. Follow
   [Releasing](RELEASING.md) to verify a fresh public tagged checkout and Docker
   build, HTTP/shutdown smoke, and the source archive.
3. Publish the GitHub **prerelease**, recording the full commit, CI URL, smoke
   receipt, installation result, and platform limitations. Verify signed-out
   access before announcing it.

Record the completed sequence in the alpha.4 GitHub prerelease notes without
changing the immutable release commit to insert its own SHA. Until those results
exist, the implementation baseline is not final release evidence. Earlier
releases remain available; this release has no hosted deployment.

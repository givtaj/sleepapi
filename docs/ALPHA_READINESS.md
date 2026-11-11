# SleepAPI alpha readiness

Assessment date: 2026-11-10 (Europe/Madrid). Current candidate:
**`0.1.1-alpha.5` — unreleased**. This candidate adds
[read-only server metadata and a capability-check client](server-metadata.md),
alongside development coverage since alpha.4. Distribution is source plus a
Dockerfile; users build their own image with Docker's machine defaults.
The planned source-prerelease date is **11 November 2026 (Europe/Madrid)**;
publication remains on hold.

[PR #20](https://github.com/givtaj/sleepapi/pull/20) was reviewed and merged on
10 November 2026 as
[`0646bfabe75e3cb29d48883e13057635c176a770`](https://github.com/givtaj/sleepapi/commit/0646bfabe75e3cb29d48883e13057635c176a770).
The merged tree matches the reviewed candidate
`856e01e8165c1f71add9a1869a6b73fad8a816c0`.
[Main CI run 42817521918](https://github.com/givtaj/sleepapi/actions/runs/42817521918)
passed all four jobs on that merge: Rust stable, Rust 1.94.0, native Linux AMD64
Docker and native Linux ARM64 Docker. The suite has 96 Rust and 37 Python tests;
eight publication-tool tests were removed with the image-publication tooling.

Documentation reconciliation in
[PR #21](https://github.com/givtaj/sleepapi/pull/21) merged as
`de5627693c1ac24372bf209110d9acd2c11d1585`, with all four jobs passing in
[main CI run 42823594104](https://github.com/givtaj/sleepapi/actions/runs/42823594104).
Its [clean-source installation](release-evidence/2026-11-10-alpha5/README.md)
also passed: anonymous commit-addressed source, all 551 files matched, a fresh
Linux ARM64 build, metadata, HTTP/recovery/replay, fixtures, faults and shutdown.
No runtime or packaging blocker was found in those scoped checks.

Source integration and candidate-installation verification are complete.
**Alpha.5 remains unreleased:** no alpha.5 tag, source release or Docker images
were published. Release-documentation review, CI and merge are authorized;
tagging, fresh tagged-source installation and publication remain later steps in
[Releasing](RELEASING.md). The final documentation commit requires its own green
main CI before any separately authorized publication. The Dockerfile is retained for
local builds using Docker's defaults. There is no image-publication workflow;
any future approved release distributes source and a Dockerfile.

The published [alpha.4 source release](https://github.com/givtaj/sleepapi/releases/tag/v0.1.1-alpha.4)
remains the quickstart/fallback for a local Docker build. Its `/metadata` route
returns `404`; that does not mean its existing recovery or fixture features are
unsupported. Historical alpha.4 image-publication evidence remains in its
release record. This candidate does not alter old tags or delete existing
registry artifacts.

## Historical background before alpha.5

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
([#6](https://github.com/givtaj/sleepapi/issues/6)) shipped in
alpha.4; see the [API contract](response-fixtures.md). Alpha.3's
historical release evidence below covers its original 58-test source only.

## Alpha.5 candidate scope

- Local developer tool, source plus Dockerfile. Users run `docker build` with
  their machine's defaults. CI tests Linux AMD64 and ARM64; macOS native
  execution is a local verification environment. Alpha.5 source publication is pending.
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
- Read-only metadata with schema/contract versions, explicit capabilities and
  configured limits; transport-fault flags depend on the actual request protocol
  and instrumented listener. The loopback client distinguishes supported,
  unsupported, and unknown without claiming verified software identity.
- No new prebuilt registry images, hosted-service, production capacity, native binary downloads, provider delivery,
  background-job, authentication, or Windows verification claim.

The 64-slot default is an admission limit, not a load-tested capacity guarantee.
Larger configurable limits remain experimental. The
[October 29 resource baseline](benchmark-results/2026-10-29-linux-arm64/README.md)
measures all four methods at concurrency 1, 16, and 64 on source `f686bad` in a
Linux ARM64 Docker Desktop VM, with separate overload/cancellation observations.
Its fixed 100-ms delay limits throughput and does not establish maximum capacity.
The [October 4 benchmarks](benchmark-results/2026-10-04-followup/README.md) remain
historical evidence on their recorded source and host.

## Release thresholds

Every required gate must pass. A skipped, missing, or stale result is pending,
not green. Fixes create a new candidate; do not relax a threshold to pass a run.

| Gate | Required threshold | Evidence |
| --- | --- | --- |
| Defects | Zero unresolved critical/high-severity defects in supported behavior: crashes, hangs, incorrect HTTP contracts, or isolation failures | Review of final diff and failure triage; this is not a claim of a security audit |
| Rust quality and contract | Formatting clean, zero Clippy warnings, all tests pass with zero ignored failures on stable and Rust 1.94.0; release builds succeed | Both Linux Rust CI jobs; current suite has 96 Rust tests, with 37 Python tests in stable CI; published alpha.4 has 86 Rust tests |
| Wait and lifecycle correctness | No early response; capacity returns structured `429` when full and recovers; dropped async work releases capacity; started blocking work retains it; graceful exit `0`, deadline/second-signal exit `2` | Rust API/wait/process tests and Docker smoke; existing process tests use a 3-second observation timeout |
| Recovery runs | Deterministic reservation order; rejected attempts consume no step; finite sequence exhaustion and stale generations reject; reset preserves lifetime budget and TTL; late completions cannot recreate cleared state | `tests/runs.rs` and recovery-inclusive Docker HTTP smoke |
| Fixtures and faults | Exact fixture bytes and allowed headers; no-header drop; byte-exact truncated prefix with incomplete declared length; rejected inputs consume no state; fault bodies retain capacity; late injection cannot restore cleared receipts | `tests/fixtures.rs`, `tests/transport.rs`, `tests/fault_lifecycle.rs`, transport unit tests and Docker client checks |
| Blocked stdout | 2,048 requests progress with stdout saturated; logger teardown and incomplete-body shutdown terminate within 1 second with 150-ms grace; second signal terminates within 1 second during a 10-second grace | Three deterministic subprocess regressions; observation limits are test guards, not general latency promises |
| Sampler shutdown | Both SIGINT and SIGTERM and a stop file finish within 1 second in the long-interval fixture; preserve samples and restore handlers on errors | Six Python regression tests; polling interval is at most 100 ms, excluding setup/I/O |
| Local HTTP regression | 64 requests on each of 4 routes at concurrency 16 and 40-ms delay: 256 valid responses, zero unexpected statuses/timeouts/transport errors; per-route p95 client overhead ≤250 ms; every health probe ≤250 ms | `scripts/alpha-smoke.py`; two Tokio workers, loopback, two-second request timeout; nearest-rank p95 includes HTTP client/network overhead |
| Admission recovery | For every method, prove capacity-one rejection during a 300-ms wait, health stays ≤250 ms while occupied, and a real simulation succeeds within 1,300 ms of the admitted attempt's launch | Separate process in alpha smoke; tests ordinary completion, while Rust tests cover cancellation semantics |
| Retry-client example | Both three-attempt sequences recover and repeat after reset; exact fixture bytes and observed faults match; stale generations consume no step; receipts correlate; owned runs are deleted | `examples/retry-client.py --binary target/release/sleepapi` in stable CI; synthetic Python-client evidence, separate from external client acceptance |
| Metadata and capability client | Bounded no-store response derived from enforced limits; read-only and responsive at full capacity; accurate HTTP/1.1/listener fault flags; strict boolean capabilities and supported/unsupported/unknown results; no verified-identity claim | Rust metadata regressions, Python capability-client tests, and `scripts/docker-smoke.py IMAGE --metadata` in both native Docker CI jobs; older alpha.4 must be checked without the flag |
| Distribution | All four CI jobs pass for the final release commit; clean package builds; both native Linux container architectures pass HTTP/CORS/non-root/shutdown and metadata smoke | All four main CI jobs passed at `de56276`; its [clean-source Linux ARM64 installation](release-evidence/2026-11-10-alpha5/README.md) passed. The release-documentation commit still requires its own exact-commit CI, followed by an approved immutable tag, signed-out source access and fresh tagged-source Docker build with metadata smoke. Publication remains pending; no registry publication |

The numeric latency limits are broad regression guards for the fixed smoke
workload on the recorded host. They are not service SLOs or maximum throughput
measurements. The October 29 CPU/RSS/thread counts provide a reproducible,
descriptive baseline for their recorded setup; they do not introduce release
budgets or production SLOs.

## Development coverage after alpha.4

The current candidate has 96 Rust and 37 Python tests. Metadata
coverage checks declarations, configured limits, listener/protocol context,
capacity independence, and unchanged run state. Capability-client tests include
older-server `404`, unknown schema/contract, missing or malformed fields,
additive metadata, and explicit false values. These checks are synthetic
compatibility evidence; they do not authenticate the responding artifact.

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

## Alpha.4 implementation baseline — 2026-10-29 (historical)

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
These results validated the implementation baseline. At that assessment, the
later alpha.4 version and documentation commit still needed its own CI and
fresh tagged-source installation before publication. The completed release
record is in the [alpha.4 prerelease notes](https://github.com/givtaj/sleepapi/releases/tag/v0.1.1-alpha.4).
This historical baseline does not validate the alpha.5 candidate.

## Alpha.3 preparation evidence (historical)

The merged implementation and documentation baseline,
`e325cc94a6b0bb3ca0d4c153a88a79c8dc88ff50`, passed
[all four CI jobs](https://github.com/givtaj/sleepapi/actions/runs/40708257954).
That evidence covered recovery runs and their completed release review before
the alpha.3 version and documentation update. It does not validate later candidates.

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

## Current next step

The planned source-prerelease date is 11 November 2026 (Europe/Madrid). Require
independent review and all four main CI jobs on the release-documentation merge,
then retain that exact SHA and CI URL in the PR completion evidence. The clean
candidate install above does not replace this final-commit gate.

Keep alpha.5 unpublished until tagging and source publication are separately
authorized. Follow [Releasing](RELEASING.md#release-sequence) for the new tag,
fresh tagged-source installation and public-access checks. Any subsequent edit,
including a changed planned release date, requires checks on its final commit.
Do not retag alpha.4 or publish Docker images. Keep the source-build quickstart
pinned to alpha.4 until the new source release exists and its installation is
verified.

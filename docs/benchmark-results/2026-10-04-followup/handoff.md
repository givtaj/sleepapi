# SleepAPI benchmark follow-up — 2026-10-04

Operation: `sleepapi-followup-2026-10-04-01a0ef83`.

User authorized the fixes and follow-up experiments identified by the saved benchmark review. Required topology: SleepAPI inside Docker on this Linux workspace; HTTP experiments executed by the rusta-pitest project agent.

## Ownership and current state

- Coordinator chat: `01a0ef83-b8a2-75e1-b397-3687c2367b21`, SleepAPI Docker setup, resource sampler, independent review, lifecycle cleanup and evidence index.
- Active runner project chat: `01a10695-28e8-7233-af63-23668f895f99`, owns rusta-pitest implementation, tests, benchmark traffic and final measurement report. Selected because it is already changing runner code for JSON output; no competing agent edits those files.
- Source: SleepAPI `d6e10c3012b042e825831606424c41437e20a06c`, unchanged application source. Reuse the verified immutable local image and verify its executable hash in every created container.
- Execution state: completed. Runner agent executed all 48 resource runs (13,260 successful measured responses), 60 contract assertions and four intentional timeout/admission-recovery checks. Coordinator stopped all five owned containers with exit code 0; image and stopped containers retained. See README.md and docker-cleanup.json.
- Verification state: passed. Docker identity/configuration, sampler checks, runner formatting/Clippy/46 Rust tests/two Python tests, native accounting and contract outcomes, raw CPU/RSS/thread recomputation and four recovery sequences verified. independent-verification.json records 768 verified artifact hashes and 3,094 samples.

## Required changes

1. Reject custom statuses `204,205,206,226,304,407,416,426` in the SleepAPI profile before traffic. Align documentation and regression tests; leave generic HTTP status assertions usable.
2. Add explicit contract validation for SleepAPI JSON and relevant response headers. Detect wrong method, wrong delay/status/scenario, malformed or incorrect body, and simulated versus actual capacity rejections. Preserve bounded memory and failure accounting; document whether assertions cover every measured response or a separate validation phase.
3. Measure CPU, process RSS and live OS thread count while varying concurrency; verify actual simulation admission after client timeouts.

## Docker targets

See `docker-setup.json` for full identities, PIDs, commands and initial metadata. Each method gets a separate server process so blocking-pool growth from one method cannot contaminate another method's process metrics.

| Role | Base URL | Route | Max active simulations |
| --- | --- | --- | ---: |
| Tokio | http://127.0.0.1:3110 | /sleep/tokio | 64 |
| Spawned Tokio | http://127.0.0.1:3111 | /sleep/tokio-spawn | 64 |
| Thread sleep | http://127.0.0.1:3112 | /sleep/thread | 64 |
| Thread park | http://127.0.0.1:3113 | /sleep/park | 64 |
| Recovery/overload | http://127.0.0.1:3114 | All four, sequentially | 1 |

All ports bind only 127.0.0.1. Per-request INFO logging is disabled with `RUST_LOG=sleepapi=warn`, which differs from the original baseline. Containers share the host without explicit CPU or memory isolation. Only one measured workload runs at a time; no builds overlap measurements.

## Resource sweep

For each method: concurrency 1, 4, 16 and 64; 400 ms requested delay; timeout 5000 ms. Three measured repetitions of a 5-second request window for each combination, with a brief recorded warmup outside measurement. Record request accounting, actual duration including drain, percentile latency, expected-status throughput and native JSON output when available.

Sample the exact container init process at 100 ms intervals using `scripts/benchmark-resources.py`. Record raw process CPU ticks, monotonic time, process RSS and live OS threads, including an idle baseline. CPU 100% means one logical CPU; RSS is process resident memory, not all Docker/cgroup memory. Align samples with recorded benchmark start/end; do not mix warmup/idle time into measured CPU averages. Include sample count and observed sampling gaps. Fresh processes isolate methods; repeats may retain thread pools/allocations within a method. Use recorded repetitions to assess spread, not declare a winner from tiny changes. At concurrency 1 a five-second window has few samples; p99 is not robust.

## Contract and recovery checks

Run all multi-route contract profiles against the dedicated capacity-1 server on port 3114 with concurrency 1 and small request counts, before recovery checks. Do not exercise other methods on the four resource containers; that would contaminate their thread-pool baselines. Run bounded contract assertions for all four methods and the simulated error scenarios. Include a real capacity rejection at the dedicated capacity-1 container, verifying `simulated:false` and `error.code:capacity_exceeded` versus simulated `rate_limited`.

For each waiting method sequentially on capacity-1:

1. Confirm an ordinary zero-delay simulation can be admitted.
2. Start a 3000 ms request with a 500 ms client deadline. While it is active, probe admission and verify an actual capacity rejection, establishing that the long request acquired the slot.
3. Record the client timeout, then poll a zero-delay simulation at a bounded interval until it returns a valid simulated 200 or a bounded deadline expires. Record admission recovery time relative to original request start and client timeout.
4. Health may be observed separately but is not the recovery criterion. Do not infer handler cancellation solely from TCP disconnect; report observed behavior. Started blocking work is expected to retain its slot until completion.
5. Ensure recovery before moving to the next method. Leave container lifecycle to the coordinator.

Save exact commands, scripts, source and binary fingerprints, native JSON reports, resource JSONL and assertion results in a new runner evidence directory. Do not mutate finalized September 30 benchmark evidence. Coordinator will verify results and stop only these owned containers after completion.

## Historical integration review of initial execution scaffold (resolved)

The initial runner script was written before the Docker/sampler protocol arrived. Before executing, adapt its one-origin restriction to the four explicit route-to-origin assignments above. Its sampler adapter must select the matching container per row. Add the authorized warmup/idle baseline to the plan; the initial `warmup:none` and excluded-warmup fields are superseded by this protocol. A continuous sampler subprocess must be stopped and reaped in `finally`, with its own monotonic samples aligned to benchmark boundaries. Contract/recovery checks must be implemented before reporting this follow-up complete. These are known preparation gaps, not observed runtime failures.

The runner owner may record the final release binary/source hash after its own build and tests and construct the machine-readable readiness snapshot from this verified server handoff. Existing user authorization and Docker readiness cover that step; no additional coordinator or user permission is needed. Keep exact source/binary/container/sampler hashes in results.

## Completion

All preparation gaps below the initial scaffold review were resolved before traffic. The runner saved execution/readiness snapshots, warmup/idle boundaries, per-case resource evidence and bounded contract/recovery results. Coordinator independently audited the evidence and completed Docker cleanup. The original readiness snapshot predates these completion updates; its hash is retained in the runner inputs. Final results and interpretation are in README.md.

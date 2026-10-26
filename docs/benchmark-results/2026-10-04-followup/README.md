# Docker benchmark follow-up — 2026-10-04

This directory records the historical Docker follow-up completed on 2026-10-04. The original report states that **all 48 resource runs passed: 13,260 measured responses returned HTTP 200 with no unexpected statuses, timeouts or transport errors. All 60 strict response-contract checks passed. Four intentional client timeouts were followed by verified simulation admission recovery.**

The original JSON receipts, identifiers and numerical results are retained. The documentation now distinguishes those recorded outcomes from current behavior; no benchmark or measurement audit was rerun for this documentation reconciliation. Complete raw runner inputs remain outside this repository, so the retained files alone cannot reproduce the original audit.

## Changes completed for the historical run

- Fixed rusta-pitest's SleepAPI status parser to reject all eight unsupported statuses before traffic: `204,205,206,226,304,407,416,426`.
- Added opt-in `--validate-contract` with a 64 KiB retained-body limit, response/header assertions, separate terminal/JSON failure counts and a failed exit when assertions fail. This distinguishes intentional simulated rate limiting from actual capacity exhaustion.
- Added SleepAPI's [read-only Linux resource sampler](../../../scripts/benchmark-resources.py) and usage documentation. No SleepAPI application source change was needed for that experiment. The linked sampler follows the current checkout and has changed since the recorded run.
- Formatting, Clippy, 46 Rust tests and two Python tests passed in the runner project. Sampler fixture, CPU arithmetic, real-container, bounds, overwrite, signal and stop-file checks passed; see [sampler validation](sampler-validation.json).

## What the recorded measurements show

Each method used a separate capacity-64 Docker process. Concurrency 1, 4, 16 and 64 each had three five-second request windows with a 400 ms simulated delay. Measured traffic was sequential, with separately recorded idle and server warmup phases. The following summarizes concurrency 64 across three repetitions:

| Method | Median successful responses/s | Median process CPU, one core = 100% | Peak live process threads | Peak process RSS |
| --- | ---: | ---: | ---: | ---: |
| Tokio timer | 156.61 | 2.83% | 6 | 7.12 MiB |
| Spawned Tokio timer | 157.21 | 3.08% | 6 | 7.12 MiB |
| Thread sleep | 157.00 | 4.23% | 71 | 7.88 MiB |
| Thread park | 156.98 | 4.34% | 81 | 7.88 MiB |

The 400 ms delay implies a concurrency-64 ceiling near 160 responses/s. All methods approach that ceiling; these results do not establish a throughput winner. The clear resource difference is OS thread growth: async methods stayed at six live process threads throughout the sweep, while blocking methods grew their pools. Thread counts include runtime/control threads and retained idle workers, not just active waits. Later low-concurrency runs can retain workers from earlier high-concurrency runs; that alone is not a leak.

The original report supported the async default for ordinary simulated latency and blocking methods as explicit comparison tools. No server performance defect was demonstrated by these measured workloads; they do not establish correctness or capacity for later code or other workloads.

## Capacity recovery after client timeout

On a separate capacity-one container, each route first admitted an ordinary simulation. A 3000 ms request then occupied the slot, proven by a valid `429 capacity_exceeded` response with `simulated:false`. The client had a 500 ms deadline; subsequent probes required a valid zero-delay simulation response to count as recovered.

| Method | First accepted probe completed after observing client-process exit | First accepted probe completed after client-process launch |
| --- | ---: | ---: |
| Tokio timer | 4.2 ms | 0.540 s |
| Spawned Tokio timer | 6.1 ms | 0.546 s |
| Thread sleep | 2.526 s | 3.061 s |
| Thread park | 2.516 s | 3.055 s |

The async methods admitted the first post-timeout probe in this tested setup. These are external process/probe observations, not exact cancellation timings or a guarantee for every HTTP disconnect. Blocking work correctly retained capacity until its original wait completed. Its conservative last-rejection-to-first-success intervals were 2.956–3.061 seconds and 2.950–3.055 seconds after process launch. `/health` was not used as proof of recovery.

## Scope and historical verification

This was a shared four-vCPU Linux host, without container CPU/memory isolation. Request INFO logging was disabled, unlike the September baseline. Warmup used a separate runner process and warmed server state; measured client connections were new. Resource windows used monotonic process boundaries including invocation/report overhead; the historical audit recomputed CPU from raw ticks strictly within each window, excluding idle/warmup and recording unsampled edges. Maximum observed sampling gap was 132.3 ms. RSS excludes reserved virtual stacks and other cgroup memory. Short runs and only 13 latency samples per concurrency-one run limit tail-latency conclusions. See [interpretation criteria](interpretation.md).

The retained [independent audit receipt](independent-verification.json) records verification of all 48 combinations, 768 saved artifact hashes, 3,094 resource samples, response accounting, route/container/PID identities, phase separation and CPU/RSS/thread arithmetic. It also records checks of all 60 contract results and the four occupied-slot → timeout → admitted-simulation sequences. Its `passed` field reports the historical audit outcome.

The original [audit script](verify-evidence.py) is included. It references the original Linux workspace paths and requires raw HTTP reports, per-run resource samples, execution receipts and contract/recovery artifacts from the runner project. Those complete inputs are not bundled here. The script writes a new receipt exclusively and intentionally refuses to overwrite an existing one; its presence alone does not make this directory a self-contained rerun.

The original report referenced these external runner artifacts:

- [Runner report and raw evidence](https://github.com/givtaj/rusta-pitest/blob/fd56bb89622dd40d59dcc398eee5fb653c29daa1/docs/benchmark-results/sleepapi-followup-2026-10-04-01a0ef83/report.md)
- [Runner implementation/build receipt](https://github.com/givtaj/rusta-pitest/blob/fd56bb89622dd40d59dcc398eee5fb653c29daa1/docs/benchmark-results/sleepapi-followup-2026-10-04-01a0ef83/implementation-ready.json)

Retained local records:

- [Docker identities and startup configuration](docker-setup.json)
- [Historical audit result rows and recovery intervals](independent-verification.json)
- [Original audit script](verify-evidence.py)
- [Sampler validation receipt](sampler-validation.json) and [idle sampler JSONL](sampler-idle-validation.jsonl)
- [Ownership and protocol](handoff.md)
- [Container cleanup receipt](docker-cleanup.json)
- [Interpretation criteria](interpretation.md)
- [Checksums of the nine payload files](SHA256SUMS.json)

SleepAPI source used in the recorded run: `d6e10c3012b042e825831606424c41437e20a06c`. Measured runner source: `7896e93fe851b3605c1cecc08688afc4169c5155` (clean release build), binary SHA256 `3f691b8523673ee3cfaa3d24f78a167a70e98c650b4b966e7c5926a5256b92da`. Later evidence-only commits do not change the measured executable. Original September evidence remains intact.

The historical [cleanup receipt](docker-cleanup.json) records all five coordinator-owned benchmark containers stopping cleanly with exit code 0 after the runner confirmed all clients and samplers had exited. It records that the stopped containers and image were retained at that time and unrelated services were untouched. Their present availability has not been rechecked.

`SHA256SUMS.json` covers the nine retained payload files in this directory, including this README and the interpretation document, and excludes the manifest itself. Its refreshed hashes describe the current file bytes after documentation reconciliation; they do not rerun or validate the historical measurements.

# Docker benchmark follow-up — 2026-10-04

This directory preserves a **historical summary, not a complete reproducible benchmark archive**. The original report stated that all 48 resource runs passed: 13,260 measured responses returned HTTP 200 with no unexpected statuses, timeouts or transport errors; 60 strict response-contract checks passed; and four intentional client timeouts were followed by simulation admission recovery.

The [retained audit summary](independent-verification.json) contains the 48 per-run result rows and four recovery intervals. Raw HTTP reports, resource samples, runner source/build receipts, the original audit script, sampler-validation receipts, handoff notes and cleanup receipts are **not included** in this repository. Their results cannot be independently reverified here. The numerical historical results below are unchanged; no benchmark was rerun while preparing this public bundle.

The [current Linux resource sampler](../../../scripts/benchmark-resources.py) and [usage documentation](../../../README.md#develop) are available for new measurements. The current sampler may differ from the version used in this historical run; its presence does not make the old raw inputs available.

## Reported measurements

Each method used a separate capacity-64 Docker process. Concurrency 1, 4, 16 and 64 each had three five-second request windows with a 400 ms simulated delay. Measured traffic was sequential, with separately recorded idle and server warmup phases. The following summarizes concurrency 64 across three repetitions:

| Method | Median successful responses/s | Median process CPU, one core = 100% | Peak live process threads | Peak process RSS |
| --- | ---: | ---: | ---: | ---: |
| Tokio timer | 156.61 | 2.83% | 6 | 7.12 MiB |
| Spawned Tokio timer | 157.21 | 3.08% | 6 | 7.12 MiB |
| Thread sleep | 157.00 | 4.23% | 71 | 7.88 MiB |
| Thread park | 156.98 | 4.34% | 81 | 7.88 MiB |

The 400 ms delay implies a concurrency-64 ceiling near 160 responses/s. All methods approach that ceiling; these results do not establish a throughput winner. The clear resource difference is OS thread growth: async methods stayed at six live process threads throughout the sweep, while blocking methods grew their pools. Thread counts include runtime/control threads and retained idle workers, not just active waits. Later low-concurrency runs can retain workers from earlier high-concurrency runs; that alone is not a leak.

The existing async default is appropriate for ordinary simulated latency. Keep blocking methods as explicit comparison tools. No server performance defect was demonstrated by these measurements.

## Capacity recovery after client timeout

On a separate capacity-one container, each route first admitted an ordinary simulation. A 3000 ms request then occupied the slot, proven by a valid `429 capacity_exceeded` response with `simulated:false`. The client had a 500 ms deadline; subsequent probes required a valid zero-delay simulation response to count as recovered.

| Method | First accepted probe completed after observing client-process exit | First accepted probe completed after client-process launch |
| --- | ---: | ---: |
| Tokio timer | 4.2 ms | 0.540 s |
| Spawned Tokio timer | 6.1 ms | 0.546 s |
| Thread sleep | 2.526 s | 3.061 s |
| Thread park | 2.516 s | 3.055 s |

The async methods admitted the first post-timeout probe in this tested setup. These are external process/probe observations, not exact cancellation timings or a guarantee for every HTTP disconnect. Blocking work correctly retained capacity until its original wait completed. Its conservative last-rejection-to-first-success intervals were 2.956–3.061 seconds and 2.950–3.055 seconds after process launch. `/health` was not used as proof of recovery.

## Scope and evidence limits

The original report describes a shared four-vCPU Linux host, without container CPU/memory isolation. Request INFO logging was disabled, unlike the September baseline. Warmup used a separate runner process and warmed server state; measured client connections were new. Resource windows used monotonic process boundaries including invocation/report overhead. The original audit reported recomputing CPU from raw ticks strictly within each window, excluding idle/warmup and recording unsampled edges. The retained summary reports a maximum sampling gap of 132.3 ms. RSS excludes reserved virtual stacks and other cgroup memory. Short runs and only 13 latency samples per concurrency-one run limit tail-latency conclusions. See [interpretation criteria](interpretation.md).

The historical audit summary reports checking 48 combinations, 768 saved artifact hashes and 3,094 resource samples, plus response accounting, route/container/PID identities, phase separation, CPU/RSS/thread arithmetic, 60 contract results and four occupied-slot → timeout → admitted-simulation sequences. Those counts describe the unavailable original inputs. The `passed` field is the historical audit's reported outcome, not a new verification of those inputs or of the current sampler.

Included public records:

- [Reported per-run values and recovery intervals](independent-verification.json)
- [Redacted historical Docker setup](docker-setup.json)
- [Interpretation criteria](interpretation.md)
- [Checksums of these public files](SHA256SUMS.json)

SleepAPI source used in the reported run: `d6e10c3012b042e825831606424c41437e20a06c`. The setup record retains source and executable hashes for identification, but it is not a fresh build receipt. Local run/session identifiers have been removed, as recorded in each JSON file's `publication` metadata. The setup describes historical containers and makes no claim that they remain available.

The checksum manifest covers exactly the four included payload files, excluding itself. It checks the integrity of this edited public bundle; it does not validate the historical measurements. From the repository root, verify it with Python 3:

```sh
python3 - <<'PY'
import hashlib
import json
from pathlib import Path

bundle = Path("docs/benchmark-results/2026-10-04-followup")
manifest = json.loads((bundle / "SHA256SUMS.json").read_text())
payloads = {p.name for p in bundle.iterdir() if p.is_file() and p.name != "SHA256SUMS.json"}
assert set(manifest) == payloads, "Manifest does not match the included files"
for name, expected in manifest.items():
    actual = hashlib.sha256((bundle / name).read_bytes()).hexdigest()
    assert actual == expected, f"Checksum mismatch: {name}"
print("Public file checksums match; historical measurements were not reverified.")
PY
```

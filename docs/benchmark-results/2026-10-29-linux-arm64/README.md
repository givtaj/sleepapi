# Linux ARM64 resource baseline — 2026-10-29

All **48 cases passed** on source
[`f686bad93ca9a337817c92f7b7ecf6becb2915ea`](https://github.com/givtaj/sleepapi/commit/f686bad93ca9a337817c92f7b7ecf6becb2915ea),
after PR #14: 36 normal-load cases and 12 overload/cancellation cases. The normal
measurement phases completed **92,826 valid responses with zero unexpected
statuses, transport errors, timeouts, or early responses**. This is a new run on
Linux ARM64 inside Docker Desktop, with 100-ms simulated waits.

The clearest resource difference is thread use. At concurrency 64, both async
methods remained at **5 process threads**, while `thread` and `park` reached
**69**. Median server CPU was about **5.2–5.3% of one core** for async methods and
**7.1–7.3%** for blocking methods. Sampled peak RSS stayed below **6.5 MiB** for
all methods. Throughput and latency were similar under this delay-limited load;
small differences do not establish a general speed ranking.

## Setup and interpretation

- Docker Desktop Linux VM: ARM64, kernel `7.0.14-linuxkit`, Engine `29.8.1`,
  10 guest logical CPUs, 8,319,504,384 bytes of guest memory.
- Server: two Tokio workers, guest CPUs `2,3`, 512 MiB memory limit, no additional
  swap, 256-PID limit, request INFO logging disabled. Capacity 64 for load and 1
  for lifecycle checks. Each case starts a fresh server.
- Client/sampler: Python 3.12.14, guest CPUs `0,1`, the same memory/PID limits,
  separate container sharing only that server's PID/network namespaces. No host
  namespace, Docker socket, privileged container, or published port.
- Each load case: 1 s idle, 2 s warmup plus drain, then a 10 s shared launch window
  plus drain, using persistent HTTP/1.1 connections. Three repetitions per cell;
  method/concurrency order shuffled within each repetition before execution.
- CPU is process CPU from raw Linux ticks, with 100% equal to one logical core.
  RSS is resident process memory, not total cgroup memory or reserved virtual
  thread stacks. Threads include runtime, logging/control, and idle pool threads.

See the [protocol](protocol.md), [environment](evidence/environment.json),
[execution plan](evidence/plan.json), and [build provenance](evidence/provenance.json).
The whole sweep took 535.7 seconds, starting at 18:50:51 UTC. No other Docker
containers were running at its start; only one case ran at a time. Guest CPU sets
do not reserve physical Mac cores, and the VM shares the host's power/thermal and
other system activity. This is a baseline for this setup, not bare-metal Linux,
maximum throughput, production capacity, or external client acceptance.

## Normal-load results

Each entry is **median (minimum–maximum)** across the three repetitions. RSS and
threads summarize each repetition's sampled peak; latency summarizes each
repetition's nearest-rank p95. Responses/s uses all completed measured requests
divided by phase start through the last response, including drain time.

| Concurrency | Method | Responses/s | p95 latency, ms | Server CPU, % of one core | Peak RSS, MiB | Peak threads |
| --- | --- | --- | --- | --- | --- | --- |
| 1 | tokio | 9.55 (9.53–9.56) | 107.28 (107.24–107.32) | 0.40 (0.40–0.40) | 3.53 (3.53–3.53) | 5 (5–5) |
| 1 | tokio-spawn | 9.54 (9.53–9.55) | 107.54 (107.24–107.58) | 0.40 (0.40–0.50) | 3.53 (3.53–3.53) | 5 (5–5) |
| 1 | thread | 9.59 (9.58–9.62) | 106.20 (106.19–106.25) | 0.51 (0.40–0.51) | 3.53 (3.53–3.53) | 6 (6–6) |
| 1 | park | 9.58 (9.58–9.59) | 106.20 (106.18–106.25) | 0.40 (0.40–0.50) | 3.53 (3.53–3.53) | 6 (6–6) |
| 16 | tokio | 151.56 (151.54–151.81) | 109.49 (108.77–109.93) | 1.92 (1.20–1.92) | 4.03 (4.03–4.03) | 5 (5–5) |
| 16 | tokio-spawn | 151.73 (151.58–152.49) | 109.60 (108.24–109.93) | 1.82 (1.21–1.82) | 4.03 (4.02–4.03) | 5 (5–5) |
| 16 | thread | 152.80 (152.59–153.96) | 108.54 (107.35–108.90) | 2.22 (1.52–2.32) | 4.28 (4.16–4.28) | 21 (21–21) |
| 16 | park | 153.01 (152.88–154.02) | 108.64 (107.33–108.95) | 2.22 (1.50–2.22) | 4.28 (4.28–4.28) | 21 (21–21) |
| 64 | tokio | 606.26 (605.67–606.40) | 111.70 (111.35–111.88) | 5.15 (4.55–5.25) | 5.41 (5.41–5.53) | 5 (5–5) |
| 64 | tokio-spawn | 604.59 (603.99–605.34) | 111.97 (111.81–112.01) | 5.30 (5.26–5.46) | 5.28 (5.28–5.65) | 5 (5–5) |
| 64 | thread | 607.11 (601.90–611.98) | 111.35 (110.24–111.74) | 7.10 (6.97–7.40) | 6.28 (6.27–6.41) | 69 (69–69) |
| 64 | park | 609.98 (608.51–612.75) | 111.27 (109.43–111.71) | 7.27 (7.20–7.30) | 6.16 (6.15–6.16) | 69 (69–69) |

The delay-only ceilings are 10, 160, and 640 responses/s. These cases approach
those ceilings and do not find saturation. The [aggregate data](aggregate.json)
also retains completions inside the fixed 10 s window per second, per-cell
counts, p50/max latency, and sampling bounds. Per-repetition metrics and every
request observation remain in the [execution record](evidence/execution.json)
and case directories.

Idle RSS was 3.52–3.53 MiB, with 5 threads in every load case. The client/sampler
averaged 1.20–17.41% of one core during measurement, so its average CPU use does
not indicate saturation; its scheduling and instrumentation still contribute to
observed latency. Server CPU counters have 10-ms resolution. Measured server
sampling gaps reached 117.76 ms; leading and trailing unsampled edges reached
103.84 and 100.55 ms respectively. CPU spans use only the first and last samples
inside the response interval, and brief RSS/thread peaks can be missed.

## Overload and cancellation

Each method passed all three capacity-one repetitions. All **192** requests in
the overload bursts returned real `429 capacity_exceeded` responses with the
expected retry header. Health succeeded while occupied, and a simulation was
admitted after the long request completed.

For cancellation, an admitted 1,500-ms wait had a 300-ms client timeout. The socket
was then closed and zero-delay simulations were polled every 10 ms. Values below
are median (minimum–maximum) times until the **first successful probe completed**.
They include socket-close handling, probe scheduling, and response time.

| Method | Recovery after observed timeout, ms | Recovery after request start, ms | Health during overload, ms |
| --- | --- | --- | --- |
| tokio | 2.46 (2.28–2.50) | 305.35 (305.12–308.47) | 0.128 (0.119–0.133) |
| tokio-spawn | 2.77 (2.10–2.82) | 304.77 (303.91–305.36) | 0.127 (0.121–0.131) |
| thread | 1204.53 (1199.72–1205.38) | 1506.07 (1501.21–1507.46) | 0.119 (0.113–0.132) |
| park | 1203.40 (1199.66–1203.50) | 1504.17 (1502.06–1505.54) | 0.128 (0.127–0.134) |

Both async methods admitted the first post-timeout probe. There is therefore no
post-timeout rejection lower bound for them. Blocking methods continued rejecting
until close to the original 1,500-ms completion. Each observation retains the
last rejected and first accepted probe's start/end timestamps in
[aggregate.json](aggregate.json) and the raw records; this is an observed recovery
interval, not an exact internal permit-release timestamp. These results agree
with started blocking work retaining capacity after its caller disconnects.

## Retained evidence and offline audit

[audit.json](audit.json) independently recomputes request accounting, latency,
throughput, CPU tick arithmetic, RSS/thread peaks, sampling gaps, and lifecycle
observations. It checks the complete matrix, source/build/frozen-harness hashes,
server identity, namespace linkage, configuration, and cleanup receipts. It does
not import the measurement worker. The SHA-256 inventory covers every retained
evidence file. The audit validates observations and arithmetic, not unseen wire
bytes or a repeat execution of the server.

The audit passed all 48 cases, **112,534 HTTP observation records**, **5,052 paired
server/client resource samples**, and 96 container identities. The
[negative-check receipt](verifier-negative-checks.json) confirms that altered
request timing, CPU counters, capacity configuration, and harness bytes are
rejected at their intended checks. All **18 Python regressions** passed,
including response framing, resource arithmetic, and failure cleanup checks.

Each of the 48 case directories retains `data/requests.jsonl.gz`,
`data/samples.jsonl.gz`, `data/phases.jsonl.gz`, the summary, worker log, server and
worker identities, and cleanup. All 144 JSONL files were compressed only after
the run, with a verified lossless round trip and original hashes in
[compression.json](evidence/compression.json). The exact source archive, build
log, and executed harness are bundled as well. All 96 owned containers exited
cleanly without OOM and were removed; the final
[container query](evidence/post-run-cleanup.json) found none remaining.

Run an independent audit without Docker or network access, writing a new receipt:

```sh
python3 -B docs/benchmark-results/2026-10-29-linux-arm64/evidence/harness/verify-linux-benchmark.py \
  docs/benchmark-results/2026-10-29-linux-arm64/evidence \
  --output /tmp/sleepapi-benchmark-audit-new.json
```

## Repeat the workload

Use an idle native Linux ARM64 Docker daemon with guest CPUs 0–3 available.
The runner refuses to start while any container is running. Build and pull
before measurement; both images are resolved to immutable IDs for execution.
Commands below run from the repository root and require new output directories.

```sh
benchmark_record=docs/benchmark-results/2026-10-29-linux-arm64
benchmark_source=$(mktemp -d)
tar -xzf "$benchmark_record/evidence/source.tar.gz" -C "$benchmark_source"
docker build --platform linux/arm64 \
  --label org.opencontainers.image.revision=f686bad93ca9a337817c92f7b7ecf6becb2915ea \
  --tag sleepapi:benchmark-f686bad "$benchmark_source"
docker pull python@sha256:78387bc3881b8273120a12ebe6c1ab22b018ccc2c9adf565ae1ac9b536e184ea
python3 -B scripts/benchmark-linux.py \
  --image sleepapi:benchmark-f686bad \
  --runner-image python@sha256:78387bc3881b8273120a12ebe6c1ab22b018ccc2c9adf565ae1ac9b536e184ea \
  --source-sha f686bad93ca9a337817c92f7b7ecf6becb2915ea \
  --output-dir /tmp/sleepapi-linux-benchmark-new --run-id new-baseline
```

The Dockerfile's Rust/Debian tags and package repositories can change. Resolved
base-image digests are retained in `build.log`; a future rebuild must record its
own provenance and binary hash, not claim identical bytes from the source SHA
alone. Retain the new runner's exact script copies under `harness/` and add its
source/build provenance before using the offline verifier.

The executed orchestrator is frozen under `evidence/harness/`. The working
`scripts/benchmark-linux.py` additionally hardens failure cleanup: an error
cleaning one owned container no longer skips the remaining containers or the
receipt. It also uses host-neutral scope metadata for future runs on Linux
daemons outside Docker Desktop. These post-run changes affect failure handling
and metadata only, with a mocked cleanup regression; they are not substituted
for the measured harness. No server source or runtime behavior changed for
this benchmark.

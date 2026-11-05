# Protocol — fresh Linux resource baseline

Selected source: `f686bad93ca9a337817c92f7b7ecf6becb2915ea`, after PR #14.
This experiment measures the four waiting implementations. It does not change
the server, certify production capacity, or constitute external client acceptance.

## Environment and isolation

Run native Linux ARM64 containers in the local Docker Desktop Linux VM. The VM
has 10 logical CPUs and approximately 8 GiB RAM. Server and runner use separate
guest CPU sets (`2,3` and `0,1`), with a 512 MiB memory limit and no additional swap
for each. Guest CPU sets do not reserve exclusive physical Apple CPU cores.
The VM still shares the Mac's hardware and power/thermal conditions.

Build the immutable source archive with the repository Dockerfile and Rust
1.94.0. Preserve the build log, source and image identities, binary hash, runner
image identity, and exact harness copies. Builds finish before measurement.
Begin with no other running Docker containers and run only one case at a time.

Each case starts a fresh server process, with two Tokio workers and request INFO
logging disabled (`RUST_LOG=sleepapi=warn`). The server runs directly as PID 1;
there is no init or shell wrapper. Its network has only loopback. The unprivileged
Python client shares the server's PID and network namespaces by full container
ID, with no host namespace, Docker socket, privileged mode, or published port.

Read only PID 1's `/proc` counters for server CPU, RSS, and threads. Verify its
command, PID namespace, start ticks, and actual CPU affinity; retain Docker's
server/runner namespace linkage and server identity before/after each case.
The existing sampler's `process_sample()` parser is reused. Its host-oriented
`inspect_container()` check is not used in this shared-namespace topology.

## Normal load

| Dimension | Setting |
| --- | --- |
| Methods | `/sleep/tokio`, `/sleep/tokio-spawn`, `/sleep/thread`, `/sleep/park` |
| Concurrency | 1, 16, 64 |
| Repetitions | 3 per method/concurrency combination; 36 total cases |
| Delay and capacity | 100 ms requested delay; 64 simulation slots |
| Phases | 1 s idle; 2 s warmup and drain; 10 s measured launch window and drain |
| Connections | One persistent HTTP/1.1 connection per worker; fresh clients after server warmup |
| Timeouts | 2 s socket timeout; bounded worker/container execution |
| Resource samples | 100 ms target interval; raw Linux monotonic time and CPU ticks |
| Order | Seeded shuffle within each repetition, fixed before measurement |

All workers use one shared launch deadline. Stop initiating requests at that
deadline, then drain outstanding work. Retain every request, including failures.
Check status, complete bounded JSON framing, simulated operation, method, delay,
blocking flag, and server/client elapsed time. Any unexpected result fails the
case; do not silently filter it from successful-response throughput.

Report total successful responses divided by elapsed time from phase start to
last response, plus completions inside the fixed launch window per second.
Compute nearest-rank p50/p95 and retain per-repetition results. The concurrency-1
case has approximately 100 measured responses; report descriptive p95, not p99
or statistical confidence intervals.

Compute mean CPU from raw counter deltas between the first and last samples
inside the measured response span. CPU 100% means one logical core. Retain
unsampled leading/trailing intervals and maximum sampling gaps. Report sampled
RSS/thread peaks and idle baselines. RSS excludes reserved virtual stacks and
other cgroup memory; sampling can miss short peaks. Thread counts include
control/runtime/logging threads and retained idle blocking workers.

The 100-ms delay gives approximate delay-only ceilings of 10, 160, and 640
responses/s. Approaching these ceilings is not a maximum-throughput benchmark.
Do not directly rank against the older 400-ms, different-host experiments.

## Overload and cancellation

For each method, run three additional fresh capacity-one processes, for 12
lifecycle cases. Each case performs both checks:

1. Admit a 1,500-ms request. Prove occupancy with a structured, non-simulated
   `429 capacity_exceeded`, then issue a bounded burst of 16 requests. Require all
   16 to be rejected with the capacity contract. Check health separately, allow
   the admitted work to complete, and prove recovery with a valid simulation.
2. Admit another 1,500-ms request with a 300-ms client timeout. Prove occupied
   capacity before observing the timeout. Close that client socket, then poll a
   zero-delay simulation every 10 ms until admission succeeds or the bounded
   recovery deadline expires. Retain the last rejection and first successful
   probe, relative to both request start and timeout observation.

Health is not evidence of restored simulation capacity. A disconnect need not
cancel every HTTP handler immediately. Started blocking work is expected to keep
its capacity slot until completion; the reported recovery interval is an
observation of this particular client/server setup.

## Evidence and cleanup

Retain request/resource/phase JSONL, per-case summaries, container identities,
execution plan, source archive, build provenance, and frozen harness. Compress
raw JSONL losslessly only after all cases finish. Independently recompute the
summaries and hash inventory from raw inputs. Stop and remove only the owned
containers, requiring exit 0, no OOM, and no restart. Preserve any failure rather
than relabeling it as a successful measurement.

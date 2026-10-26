# Interpretation criteria for the follow-up

These criteria were reviewed against SleepAPI source `d6e10c3012b042e825831606424c41437e20a06c` before interpreting the historical 2026-10-04 results. The [bundle summary](README.md) lists the retained audit script and supporting receipts, as well as the complete raw runner inputs held outside this repository. The retained files alone do not reproduce the original audit.

- At a 400 ms delay, the closed-loop delay-only throughput ceilings for concurrency 1, 4, 16 and 64 are approximately 2.5, 10, 40 and 160 responses/s. Near-ceiling results establish that these workloads complete efficiently; they do not measure maximum throughput or capacity above 64.
- `/health` bypasses simulation admission. On the isolated capacity-one server, a confirmed `429` with `simulated:false` and `error.code:capacity_exceeded`, followed by a valid zero-delay simulation response, establishes slot recovery. Probe timing gives a recovery interval rather than an exact instant.
- Report recovery relative to the original request start and the client timeout. Blocking waits deliberately retain their permit until started work finishes. Releasing that permit on disconnect would undercount active work. Async waits release capacity if their handler is dropped, but a client disconnect does not guarantee immediate handler cancellation in the HTTP stack.
- Compare CPU counter deltas over measured samples and disclose sampling-boundary gaps. Individual 100 ms CPU readings are tick-quantized. CPU at 100% means one logical CPU; only server process CPU is measured.
- Live OS thread count includes runtime threads and idle blocking-pool threads. A growing pool for blocking waits is expected. Pool threads can persist between repeats and concurrency levels within a method, so retained counts do not establish a leak.
- RSS measures resident process pages, excluding reserved virtual stacks and other cgroup memory. Report peaks and idle baselines; retained RSS alone does not prove leaked memory. Sampling can miss brief peaks.
- Separate method containers avoid cross-method pool contamination. This shared four-vCPU host has no CPU or memory isolation. Short runs, small concurrency-one sample counts, logging differences from the September baseline and unrelated host activity preclude fine-grained performance rankings.

Current source references (which may differ from the measured commit): [admission](../../../src/lib.rs), [waiting and permit ownership](../../../src/wait.rs), [documented cancellation behavior](../../../README.md), and [resource sampler](../../../scripts/benchmark-resources.py).

The historical source review reported no inaccurate cancellation/admission documentation and no server defect requiring an application change before those measurements. That conclusion applies to the reviewed source and workload; it does not validate later implementations or untested failure conditions.

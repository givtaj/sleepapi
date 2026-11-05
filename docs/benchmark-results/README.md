# Benchmark records

These directories preserve SleepAPI benchmark reports and supporting receipts.
Recorded values, identifiers, and outcomes describe each selected source,
environment, and workload; they are not general capacity guarantees.

- [October 29 Linux ARM64 resource baseline](2026-10-29-linux-arm64/README.md) measures source `f686bad` after PR #14 in Docker Desktop's Linux VM. It retains all 48 load/lifecycle cases, raw request/resource/phase records, source and frozen harness, build provenance, and an independent audit. Its 100-ms workload and host differ from the older experiments below.

- [September 30 Docker benchmark](2026-09-30-local/README.md) retains baseline and scenario output, exact command receipts, copied runner-evidence hashes, Docker setup/build/server logs, readiness probes, lifecycle observations and coordination records. The measured runner source and its original workspace are not bundled here. The preliminary native setup was superseded by the Docker run and is not evidence of a native benchmark.
- [October 4 follow-up](2026-10-04-followup/README.md) retains the historical per-run audit summary, Docker setup and cleanup receipts, sampler validation and idle samples, interpretation criteria, handoff record, original audit script and a checksum manifest. Complete raw runner HTTP reports, per-run resource samples and execution/contract/recovery inputs remain outside this repository. The README retains the original external references and explains the audit's rerun requirements.

The original receipts and historical identifiers are preserved. Fields such as `passed`, `metrics_validated`, `running` and `retained_for_rerun`, along with historical statements that containers or images were retained, describe the recorded time. Current availability has not been rechecked. A checksum match verifies retained file bytes; it does not rerun measurements or validate current source.

Use the [current sampler instructions](../../README.md#develop) to collect new resource data from a Linux Docker host. The current sampler may differ from the version used in these records. Historical measurements are observations from one setup, not performance guarantees.

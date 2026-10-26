# Historical benchmark records

These directories preserve SleepAPI benchmark reports and supporting receipts from September 30 and October 4, 2026. Recorded values, identifiers and outcomes describe those runs. They do not establish the behavior or capacity of the current application, and no benchmark was rerun for this documentation reconciliation.

- [September 30 Docker benchmark](2026-09-30-local/README.md) retains baseline and scenario output, exact command receipts, copied runner-evidence hashes, Docker setup/build/server logs, readiness probes, lifecycle observations and coordination records. The measured runner source and its original workspace are not bundled here. The preliminary native setup was superseded by the Docker run and is not evidence of a native benchmark.
- [October 4 follow-up](2026-10-04-followup/README.md) retains the historical per-run audit summary, Docker setup and cleanup receipts, sampler validation and idle samples, interpretation criteria, handoff record, original audit script and a checksum manifest. Complete raw runner HTTP reports, per-run resource samples and execution/contract/recovery inputs remain outside this repository. The README retains the original external references and explains the audit's rerun requirements.

The original receipts and historical identifiers are preserved. Fields such as `passed`, `metrics_validated`, `running` and `retained_for_rerun`, along with historical statements that containers or images were retained, describe the recorded time. Current availability has not been rechecked. A checksum match verifies retained file bytes; it does not rerun measurements or validate current source.

Use the [current sampler instructions](../../README.md#develop) to collect new resource data from a Linux Docker host. The current sampler may differ from the version used in these records. Historical measurements are observations from one setup, not performance guarantees.

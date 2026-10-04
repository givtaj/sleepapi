# Historical benchmark records

These directories contain selected historical records, not complete benchmark inputs or a verification of the current code. No benchmark was rerun to prepare the public copies.

- [September 30 records](2026-09-30-local/) retain HTTP probes, setup information, lifecycle observations and runner summary counts. The raw runner reports and runner source are not included. Statuses such as `metrics_validated`, `running` and `retained_for_rerun` describe the recorded time only. The native setup was superseded by the Docker run; its presence is not evidence of a native benchmark.
- [October 4 follow-up](2026-10-04-followup/README.md) retains result rows, reported recovery intervals and a redacted setup record. Its README lists the missing inputs and explains the limits of the public checksum manifest.

Files with a `publication` object have been edited for public distribution: unrelated project names and local run/session suffixes were removed or generalized where present. Recorded request counts, timings, resource measurements, statuses and source/executable hashes are unchanged. Container IDs retained across receipts only correlate historical observations; they do not identify a currently available service. Commands with redacted identifiers are illustrative and are not exact original invocations.

Use the [current sampler instructions](../../README.md#develop) to collect new resource data from a Linux Docker host. Historical values are observations from one setup, not performance guarantees.

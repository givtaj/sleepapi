# SleepAPI Docker benchmark — 2026-09-30

Completed on the Linux workspace, with the benchmark executed by the agent in the existing **rusta-pitest project chat**. SleepAPI ran inside Docker; the host client connected to `http://127.0.0.1:3100`. Times below use Europe/Madrid.

**Result: 400/400 baseline requests returned the expected HTTP 200, with zero unexpected statuses, timeouts, or transport errors.** The four error scenarios returned all 16 expected statuses. The intentional timeout scenario produced four timeouts and the expected failing runner exit code. Final health was HTTP 200.

## Baseline

One sequential run of each route: 100 requests, concurrency 4, configured delay 400 ms, per-request timeout 5000 ms. Baseline wall time: 40.387 seconds. Full suite including scenario checks, drain and health: 58.436 seconds, 01:49:03–01:50:02.

| Endpoint | Passed | Successful responses/s | p50 ms | p95 ms | p99 ms |
| --- | ---: | ---: | ---: | ---: | ---: |
| `/sleep/tokio` | 100/100 | 9.93 | 402.943 | 405.503 | 406.527 |
| `/sleep/tokio-spawn` | 100/100 | 9.89 | 402.687 | 405.503 | 413.183 |
| `/sleep/thread` | 100/100 | 9.93 | 401.663 | 407.039 | 416.767 |
| `/sleep/park` | 100/100 | 9.94 | 401.663 | 405.503 | 411.903 |

These are closed-loop HTTP/1.1 measurements with no dedicated warmup and no retries; connection setup is included. The server received four short readiness probes before the benchmark. Client and container shared a 4-vCPU Linux host with other workloads and no CPU isolation. The configured delay dominates these results; the small differences do not establish a performance winner or maximum capacity. CPU, memory and blocking-thread growth were not measured.

## Scenario checks

Each scenario sent one request to each route, concurrency 1, with its preset delay.

| Scenario | Server delay ms | Expected result | Observed |
| --- | ---: | --- | --- |
| `delayed_failure` | 800 | HTTP 503 | 4/4; runner exit 0 |
| `rate_limited` | 100 | HTTP 429 | 4/4; runner exit 0 |
| `invalid_request` | 50 | HTTP 400 | 4/4; runner exit 0 |
| `upstream_timeout` | 2000 | HTTP 504 | 4/4; runner exit 0 |
| `client_timeout` | 3000 | Client deadline 500 ms | 4 timeouts; zero complete responses; runner exit 1 |

The timeout profile correctly marks all four endpoints failed. Its expected nonzero exit is evidence that timeout accounting works, not an ordinary successful profile. A 3.200-second drain preceded the final health check. Total suite traffic: 420 benchmark requests plus one health request. Readiness probes are separate.

The runner asserts HTTP statuses, not error JSON or `Retry-After`. The coordinator separately verified live method/body metadata for the four routes during readiness.

## Provenance and verification

- SleepAPI: release commit `d6e10c3012b042e825831606424c41437e20a06c`; locked release build using the repository Dockerfile and Rust 1.94.0. Executed application source matched that commit.
- Docker: image `sleepapi:benchmark-20260930-01a0ef83`, container `sleepapi-benchmark-01a0ef83`, Linux amd64, user `65532:65532`. Published port: `127.0.0.1:3100` to container port `3000`.
- Container binary SHA-256: `aa947abe7e4f6f71d157896006224151d32b3d1dcc7a1a4feae5ae3cc225472e`.
- rusta-pitest: HEAD `24a7bb9c6498aec4ddce260169f9a40160bf0c34` **plus existing uncommitted profile changes**, preserved throughout the run. Binary SHA-256: `2ff4318e0365a0c7bd15fd514573dd4dc12ea74a45122bbc37b68337189a09c5`. Source manifests and dirty-diff fingerprints are retained by the runner project.
- Independent coordinator check: Docker logs show exactly 100 completions with delay 400 ms and status 200 for each method, matching the 400 client successes. An independent read-only reviewer verified output hashes, scenario counts and unchanged binary/container identity.
- Benchmark code was not changed. No builds were observed during the measured suite.

## Evidence

- [Raw baseline](runner/baseline.stdout.txt) and [exact baseline command receipt](runner/baseline.command.json).
- [Suite command receipts](runner/benchmark-commands.json), [execution summary](runner/execution-summary.json), and [timeout report](runner/client-timeout.stdout.txt).
- [Docker build log](docker-build.log), [container/source provenance](docker-provenance.json), and [readiness requests/responses](docker-contract-probes.json).
- [Independent Docker log verification](docker-log-verification.json) and [server logs](docker-server.log).
- [Hashes of copied runner evidence](runner-copy-manifest.json). Copies match the files produced by the project agent; its original directory is `/home/dev/workspace/projects/rusta-pitest/docs/benchmark-results/sleepapi-local-2026-09-30-01a0ef83`.
- [Coordination record](handoff.md).

The earlier native server preparation is superseded. No native benchmark requests were sent. Files named `server-provenance.json`, `server.log`, `server-exit.json`, and `contract-probes.json` describe only that preliminary setup; final results use the Docker evidence.

## Lifecycle and rerun

After benchmark completion, the coordinator stopped the temporary container at 01:52:24. Docker recorded exit code **0** and no OOM kill. The stopped container and local image remain available. See the [lifecycle receipt](docker-lifecycle.json). Runner metadata records the earlier state at completion, when the container was still running; this lifecycle receipt is the later authoritative state.

Restart it with:

```sh
docker start sleepapi-benchmark-01a0ef83
```

The exact baseline invocation used by the rusta-pitest agent was:

```sh
/home/dev/workspace/projects/rusta-pitest/target/release/rusta-pitest \
  profile sleepapi --base-url http://127.0.0.1:3100 \
  --requests 100 --concurrency 4 --delay-ms 400 \
  --status-code 200 --timeout-ms 5000
```

The executable hash above identifies the measured runner; rebuilding after source changes can produce different results.

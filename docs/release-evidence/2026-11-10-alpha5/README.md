# Alpha.5 clean-source candidate verification

Observed **10 November 2026, 15:09 Europe/Madrid**. Result: **PASS for the
candidate installation**, with no runtime or packaging blocker found in the
scoped checks. Alpha.5 remains unpublished.

## Verified identity and installation route

- Version: `0.1.1-alpha.5`.
- Source commit: [`de5627693c1ac24372bf209110d9acd2c11d1585`](https://github.com/givtaj/sleepapi/commit/de5627693c1ac24372bf209110d9acd2c11d1585).
- Source tree: `04e0a15e12d5c5c4cb649af1b76556db74af97b3`.
- Acquisition: anonymous HTTPS download of the [commit-addressed source archive](https://codeload.github.com/givtaj/sleepapi/tar.gz/de5627693c1ac24372bf209110d9acd2c11d1585).
- Source archive SHA-256: `4cf8004cf9c74dc745d976da45ffbf4af7bdb86172a628173fd0336ee4cda1a6`.
- Local image ID: `sha256:adbe14d99ed68cae7ab6d2bacb43e006590e1137866c16ead859f3385c97fd4a`.
- Runtime binary SHA-256: `e3c64c1435b02e13645b384d38b4d5ee584f9b5db802c24adb2aa987cb7e006c`.

All 551 extracted files matched the candidate's Git blobs, including the
Dockerfile, lockfile, source, tests and license. The fresh directory contained
no build output. `docker build --pull --no-cache` compiled with Rust 1.94.0 and
locked dependencies using Docker's default Linux ARM64 platform on macOS ARM64.
No application build layer was reused; the build log recorded fresh dependency
downloads and compilation. Docker may reuse the resolved base-image layers.

This is an **unpublished candidate check**, not an alpha.5 tagged-source install.
The source/archive match and build/image/binary records establish the tested
artifact's identity; runtime metadata alone is only a declaration. No separate
alpha.5 release signature or published artifact existed at verification time.

## Results

| Check | Result |
| --- | --- |
| Startup and declared version | PASS; non-root `65532:65532`, loopback exposure, metadata reports `sleepapi` / `0.1.1-alpha.5` |
| HTTP contract | PASS; health, seven scenarios, all four timed wait routes, CORS and 401/405 headers |
| Metadata and capability client | PASS; bounded no-store response, effective limits, fault capabilities and read-only client classifications |
| Recovery runs | PASS; sequence, receipts, exhaustion, generation-guarded reset and deletion |
| Fixtures and faults | PASS; exact UTF-8 bytes and headers, connection drop, truncated prefix, recovery and cleanup |
| Retry-client example | PASS; both status and transport recovery, two generations each, reset/replay and deleted/absent cleanup |
| Shutdown | PASS; normal exit `0`; incomplete request exits `2` after grace without Docker SIGKILL |
| Preservation and cleanup | PASS; source unchanged, no cleanup errors, no remaining candidate containers |
| Independent evidence review | PASS; source hashes, build identity, test receipts and recorded cleanup checked without rerunning tests |

The [machine-readable receipt](clean-source-install.json) records the source,
environment, build inputs, artifact identities and evidence limits.
The [Docker smoke output](docker-smoke.log) and [retry/replay record](retry-client.json)
retain the synthetic test outcomes. The retry example's `target_release` is
alpha.4 because it exercises that API contract; it is not a binary version claim.

## CI and remaining release gates

[Main CI run 42823594104](https://github.com/givtaj/sleepapi/actions/runs/42823594104)
passed all four jobs on this exact source commit: Rust stable, Rust 1.94.0,
native Linux AMD64 Docker and native Linux ARM64 Docker. That includes the
Rust/Python tests, formatting/lint, release builds, alpha regression thresholds,
retry/replay, packaging and container smoke required by the workflow.

This receipt remains bound to `de56276`. Release-documentation edits create a
new candidate requiring independent review and green CI on its exact merge
commit. Record that result in the PR completion evidence; this historical
receipt must not be relabeled as a test of a later revision.

After separate publication authorization, verify the annotated tag's identity,
signed-out tagged-source download/build, and public release access as described
in [Releasing](../../RELEASING.md#release-sequence). The planned date does not
announce a release or authorize publication. No tag, release, or registry image
was published during this verification.

Windows and native Windows remain unverified. No native macOS build or fresh
local AMD64 install was run in this check; exact-commit hosted CI covers AMD64.
There was no updater/rollback drill. These results are neither customer
acceptance nor production capacity evidence.

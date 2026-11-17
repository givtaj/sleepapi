# Alpha.5 tagged-source release verification

**PASS**, observed 17 November 2026 at 10:09 Europe/Madrid. The
[GitHub prerelease](https://github.com/givtaj/sleepapi/releases/tag/v0.1.1-alpha.5)
was published at 09:08:47 UTC (10:08:47 Europe/Madrid).
Distribution is source plus the Dockerfile for local builds using Docker's
defaults. No registry images or executable assets were published.

## Immutable release identity

- Tag: `v0.1.1-alpha.5`; annotated tag object:
  `c010a875f2d6c9a731dfa7fce0571ba4cbb7a4de`.
- Commit: [`f10bd4c9551f4617f68e9ba7aa08385e178dff6a`](https://github.com/givtaj/sleepapi/commit/f10bd4c9551f4617f68e9ba7aa08385e178dff6a).
- Tree: `89a53bf72eed09028525b5c24070aa903d8b4c40`.
- [Exact-commit main CI](https://github.com/givtaj/sleepapi/actions/runs/44051798111):
  all four jobs passed, including 96 Rust tests on each toolchain and 37 Python
  tests in stable CI, lint, builds, acceptance, retry/replay, packaging and both
  native Linux container architectures.
- [Source tar.gz](https://github.com/givtaj/sleepapi/archive/refs/tags/v0.1.1-alpha.5.tar.gz)
  SHA-256: `65b9200f08127ba609c9ae2cbaa5a8c81aef0668c36509015c4420c03635e54f`.
- [Source ZIP](https://github.com/givtaj/sleepapi/archive/refs/tags/v0.1.1-alpha.5.zip)
  SHA-256: `5d4f5bb6f659f2886a4d06074f79025614e57f4d161c7794f9feb8af6188d381`.

The fresh anonymous tagged clone and both archives matched all **555 tracked
files** against the release commit's Git blobs. Required Dockerfile, lockfile,
source and license files were present; no host build output was needed.
After publication, anonymous release-page/API access passed, and downloads
through GitHub's public archive links matched the verified archives.
Recorded digests identify these downloads; GitHub may regenerate archive bytes.
The annotated tag is unsigned. These checks establish source consistency,
not independent publisher authentication.

## Fresh installation

The tagged checkout was built with `docker build --pull --no-cache`, Rust 1.94.0
and locked dependencies on macOS 26.0.1 ARM64 with Docker Engine 29.8.1.
Docker's default platform resolved to Linux ARM64; no platform override was
used. Dependencies were downloaded and application code compiled afresh.
Base-image layers may be reused. Source files remained unchanged.

- Local image ID: `sha256:0f6d20c27c50181a7ddb6b463fa601d4cf2854080e38d4a615c4d38a913f3b26`.
- Runtime binary SHA-256: `e3c64c1435b02e13645b384d38b4d5ee584f9b5db802c24adb2aa987cb7e006c`.
- Runtime UID: `65532`; configured user: `65532:65532`; loopback-only exposure.
- Metadata declares `sleepapi / 0.1.1-alpha.5`; artifact identity is established
  separately by the source, build and image records.

| Check | Result |
| --- | --- |
| HTTP and metadata | PASS: health, seven scenarios, four timed routes, CORS, headers, bounded metadata, limits and capability client |
| Recovery and fixtures | PASS: sequence, receipt correlation, generation guards, reset/deletion and exact fixture bytes |
| Transport faults | PASS: connection drop, truncated response, recovery and cleanup |
| Retry example | PASS: status and transport cases, two generations each, reset/replay and deleted/absent owned runs |
| Shutdown | PASS: ordinary exit 0 and incomplete-request deadline exit 2 without Docker SIGKILL |
| Resource cleanup | PASS: owned verification containers removed, no new smoke containers left |
| Independent evidence review | PASS: source, archives, CI, build and installation receipts matched |
| Public source release | PASS: prerelease page/API and both source archives accessible without authentication |

The [machine-readable receipt](tagged-source-install.json),
[Docker smoke log](docker-smoke.log), and [retry-client report](retry-client.json)
retain the evidence. The retry report's `target_release` is alpha.4 because
that label describes the compatible API contract, not the running version.

The first verification attempt passed product checks but recorded a helper
cleanup error: its case-sensitive matching did not recognize Docker's lowercase
“no such object” response after removal. The failed receipt was preserved and
an independent exact-container listing confirmed absence. Only that helper
matching changed; a fresh rerun passed with no cleanup errors. Both attempts'
identities and the diagnosis are retained in the machine-readable receipt.

## Coverage limits

The fresh local installation exercised Linux ARM64 containers. Exact-commit
hosted CI covers native Linux AMD64 and ARM64 builds and smoke checks. Fresh
local AMD64 installation, native macOS/Windows builds, Windows host integration,
updater/rollback, customer acceptance and production capacity were not tested.

The [10 November candidate check](../2026-11-10-alpha5/README.md) remains bound to
`de5627693c1ac24372bf209110d9acd2c11d1585`. It is historical and is not relabeled
as acceptance of this tag. This completion record was added after publication;
the release commit and tag remain unchanged.

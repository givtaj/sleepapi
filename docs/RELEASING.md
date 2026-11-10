# Releasing SleepAPI

The current candidate is **`0.1.1-alpha.5` — merged, not yet released**.
The planned source-prerelease date is **10 November 2026 (Europe/Madrid)**.
Release documentation and its review, CI and merge are authorized; tagging and
source publication remain on hold. The date is a plan, not a publication record.
Distribution is source plus a Dockerfile for local builds using Docker's
defaults; Docker image publication is disabled.

The published [alpha.4 release](https://github.com/givtaj/sleepapi/releases/tag/v0.1.1-alpha.4)
remains the source-build fallback and README quickstart target. Its source
does not include `/metadata`.

## Distribution scope

Ship a GitHub source release containing the Rust application, lockfile, Dockerfile,
tests, and documentation. Users build their own image with `docker build` using
their machine's Docker defaults; installation commands do not override the
platform. Linux AMD64 and ARM64 containers are tested. Use Docker Engine on
Linux or Docker Desktop in Linux-container mode on macOS and Windows. A native
Rust build requires Rust 1.94 or newer.

Linux CI builds and tests containers without publishing them. This candidate
has no container-registry publication workflow: a future source release from
this candidate does not publish an image. Prebuilt registry images, native binary downloads,
crates.io publication, and native macOS/Windows CI remain outside this track.

Previously published alpha.4 source and image evidence remains historical.
Removing the publication workflow here does not change old tags, their files,
or existing registry artifacts. Do not republish an old tag to apply this policy.

## Release evidence

The reviewed alpha.5 candidate
`856e01e8165c1f71add9a1869a6b73fad8a816c0` passed
[PR CI run 42798848230](https://github.com/givtaj/sleepapi/actions/runs/42798848230).
PR #20 merged it as
[`0646bfabe75e3cb29d48883e13057635c176a770`](https://github.com/givtaj/sleepapi/commit/0646bfabe75e3cb29d48883e13057635c176a770)
with an identical tree.
[Main CI run 42817521918](https://github.com/givtaj/sleepapi/actions/runs/42817521918)
passed Rust stable, Rust 1.94.0 and both native Linux Docker jobs on that exact
merge commit. This includes 96 Rust and 37 Python tests after removal of eight
container-publication tests and their tooling. These results establish source
integration, not a release. Tagging, fresh tagged-source installation and
publication remain pending [alpha gates](ALPHA_READINESS.md). Later changes
require validation on the final commit before any separately approved release.

[PR #21](https://github.com/givtaj/sleepapi/pull/21) subsequently reconciled the
integration records as
[`de5627693c1ac24372bf209110d9acd2c11d1585`](https://github.com/givtaj/sleepapi/commit/de5627693c1ac24372bf209110d9acd2c11d1585).
All four jobs passed in its
[main CI run 42823594104](https://github.com/givtaj/sleepapi/actions/runs/42823594104).
The [10 November clean-source verification](release-evidence/2026-11-10-alpha5/README.md)
downloaded that commit anonymously, matched all 551 source files, built with
`--pull --no-cache` using Docker's default Linux ARM64 platform, and passed
metadata, HTTP, recovery/replay, fixtures, transport faults and shutdown checks.
This is candidate-installation evidence, not a tagged-release installation.

The release-documentation merge needs its own green main CI on the resulting
commit. Record that final SHA and CI URL in the PR completion evidence and,
after publication is approved, the prerelease notes. Do not create another
commit solely to insert its own SHA. If publication moves to a different date,
update the planned date and revalidate the resulting commit before tagging.

The historical verification record is in the [v0.1.0 GitHub release notes](https://github.com/givtaj/sleepapi/releases/tag/v0.1.0), published on 2026-09-30 (Europe/Madrid):
each new release must likewise record the full released commit SHA, a successful
CI run on that SHA, fresh tagged-source build results, and platform limitations.
The release tag and CI run must refer to the same commit. A local check or a workflow file alone is not release evidence.

The [alpha.1 release record](https://github.com/givtaj/sleepapi/releases/tag/v0.1.1-alpha.1)
contains its 39-test and installation evidence. Alpha.2 added three blocked-stdout
regressions, bringing the Rust suite to 42 tests; the
[alpha.3 release](https://github.com/givtaj/sleepapi/releases/tag/v0.1.1-alpha.3)
added 16 recovery-run tests for a total of 58. Alpha.4's fixture and transport
release has 86 Rust tests. Its merged implementation baseline
`535301ac7c4d2adcc6a8d33a6f121c71d5247d96` passed all four jobs in
[main CI run 40883400744](https://github.com/givtaj/sleepapi/actions/runs/40883400744).
Final alpha.4 publication evidence belongs to its
[release record](https://github.com/givtaj/sleepapi/releases/tag/v0.1.1-alpha.4).
Never recreate or move a published tag. Windows host integration and native
Windows builds have not been tested.

The container smoke script checks health, scenario discovery returning seven
entries, timed failures on all four waiting routes, CORS, protocol headers,
recovery-run sequencing, receipts, generation guards, reset and deletion,
exact fixtures, connection drops and truncated responses, normal exit `0`,
and deadline exit `2` for an incomplete request. Rust tests
also cover preset definitions, validation, concurrency, cancellation, and parking
through early wakes. These checks are functional verification, not a throughput
benchmark or proof of production-scale capacity. For alpha.5, pass `--metadata`
to also check metadata and the capability client. Omit that flag for alpha.4,
whose `/metadata` route returns `404`. Verify the image built from a fresh
tagged-source checkout before announcing the source release.

Use this checklist for each release and include the final results in its notes:

| Required check | Completion evidence |
| --- | --- |
| Release candidate committed and pushed | Full commit SHA and source link |
| Linux / Rust stable | Successful CI job including formatting, lint, 96 Rust/37 Python tests, build, alpha smoke, retry-client example, and packaging |
| Linux / Rust 1.94.0 | Successful CI job including lint, tests, and build |
| Docker / linux/amd64 | Successful native build and container smoke checks, including `--metadata` for alpha.5 |
| Docker / linux/arm64 | Successful native build and container smoke checks, including `--metadata` for alpha.5 |
| Tag matches the tested commit | Annotated tag resolves to the same full SHA |
| Fresh public installation works | New tagged checkout, local Docker build, HTTP and shutdown checks |
| Source release is publicly available | GitHub release and source archive URLs work without authentication |

## Release sequence

1. **Prepare and review one candidate.** Choose a new, unused version and match
   `Cargo.toml` and the root package in `Cargo.lock`. Keep its changelog entry
   marked unreleased until publication is scheduled. Update candidate scope and
   readiness, while retaining the currently published source tag in the quickstart.
   Open a PR and require review plus passing CI on its exact head. This is the
   stopping point for candidate preparation; it does not merge or publish.

2. **After merge approval, finalize and merge, then require green `main` CI.**
   Set the actual planned release date and reconcile publication wording before
   the final commit. Check
   [.github/workflows/ci.yml](../.github/workflows/ci.yml) on the final commit:
   Linux / Rust stable, Linux / Rust 1.94.0, Docker / linux/amd64, and Docker /
   linux/arm64 must all succeed. Record the full commit SHA and CI run URL. A
   failed job blocks the release; a fix creates a new candidate that needs CI.
   Example inspection commands:

   ```sh
   git rev-parse HEAD
   gh run list --repo givtaj/sleepapi --workflow ci.yml --branch main --limit 5
   ```

   Match the run's commit to the candidate; do not rely only on the latest green
   badge or on local test results. This workflow runs for `main` pushes and pull
   requests, not tag pushes.

3. **After tag/release approval, tag the exact green commit.** Set
   `SLEEPAPI_RELEASE_VERSION` to the chosen Cargo version and
   `SLEEPAPI_RELEASE_TAG="v${SLEEPAPI_RELEASE_VERSION}"`. Create and push that
   new annotated tag. Verify `git rev-parse "${SLEEPAPI_RELEASE_TAG}^{commit}"`
   equals the SHA recorded in step 2. Never move a published tag; a fix needs a
   new version.

4. **Verify the published source path before announcing the release.** In a
   fresh directory, use the README's version-pinned clone and Docker commands,
   substituting the chosen source tag. Build the checkout using Docker's default
   platform and a local image tag:

   ```sh
   docker build -t sleepapi:dev .
   python3 scripts/docker-smoke.py sleepapi:dev --metadata
   ```

   Confirm `GET /health` returns `{"status":"ok"}` and a POST simulation waits
   and returns its documented JSON; the smoke script checks these behaviors and
   metadata against the locally built image.

   This needs Docker and Python 3. The test creates and cleans up only its own
   temporary containers. Also check the GitHub source archive contains the
   Dockerfile, `Cargo.lock`, `src/`, and `LICENSE`; the build must not depend on
   an existing `target/` directory or local secrets.

5. **Publish the approved GitHub prerelease using the tested tag.** Use the tag as
   the title, enable the prerelease flag, summarize the changes and limits, and link the
   README's Docker instructions. [GitHub provides source archives](https://docs.github.com/en/repositories/releasing-projects-on-github/about-releases).
   Publish source and the Dockerfile, with no registry image. After verifying
   signed-out release/source access and the fresh source-build installation,
   update the quickstart's pinned source tag to the new version.

6. **Record completion.** Put the released commit, successful CI run, release URL,
   and fresh-install result in the GitHub prerelease notes linked from the
   readiness record. This avoids changing the immutable release commit merely
   to insert its own SHA. Creating a release is a publication step; updating
   these documents alone does not perform it.

## Local checks when implementation changes

Run the same checks used by CI before preparing the final release commit:

```sh
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
cargo +1.94.0 test --locked
cargo build --release --locked
python3 -B -m unittest discover -s scripts -p 'test_*.py' -v
python3 scripts/alpha-smoke.py --binary target/release/sleepapi
python3 examples/retry-client.py --binary target/release/sleepapi
cargo package --locked
docker build -t sleepapi:ci .
python3 scripts/docker-smoke.py sleepapi:ci --metadata
```

`cargo package` checks source packaging; it does not publish to crates.io. Run it
from a clean committed checkout. The full CI workflow additionally lints/builds
on Rust 1.94.0 and builds/tests the Dockerfile on both native Linux architectures.

## Publication record

This is a checklist template, not current publication status. Copy it into the
GitHub prerelease notes and fill every field from actual results:

- Candidate version and tag: pending; full commit SHA: pending.
- Exact-commit `main` CI URL and all four required job results: pending.
- Alpha smoke JSON receipt in the stable CI log: pending.
- Annotated tag target equals the tested SHA: pending.
- Fresh tagged-source Docker build and smoke, platform, date: pending.
- Signed-out release and source-archive verification: pending.
- Metadata-enabled smoke and capability-client results on the locally built image: pending.
- Distribution: source plus Dockerfile; Windows remains unverified.

A green PR run makes the candidate reviewable. After merging, require a green
`main` run for the resulting commit before tagging. Any subsequent source,
version, workflow, or documentation edit creates a new candidate. A published
release is immutable: corrections use a new version. If installation fails,
stop publication and build the published `v0.1.1-alpha.4` source tag locally
while preparing a new version.

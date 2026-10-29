# Releasing SleepAPI

## Distribution scope

Ship a GitHub source release containing the Rust application, lockfile, Dockerfile,
tests, and documentation. Users build their own image. Linux AMD64 and ARM64
containers run through Docker on Linux, macOS, and Windows with Linux-container
support. A native Rust build requires Rust 1.94 or newer.

This source-release track does not require a container registry, image-publishing
workflow, native binary downloads, crates.io publication, or native macOS/Windows CI.
Ordinary Linux CI builds and tests containers without publishing them.

## Release evidence

`v0.1.0` was published on 2026-09-30 (Europe/Madrid). Never recreate or move that
tag. The current release candidate is `0.1.1-alpha.4`; publish it as a GitHub
**prerelease** only after the [alpha gates](ALPHA_READINESS.md) pass. Its
[GitHub release notes](https://github.com/givtaj/sleepapi/releases/tag/v0.1.1-alpha.4)
will be the publication and verification record. Preparing this document does
not establish publication or completion of the final candidate's checks.

The historical verification record is in the [v0.1.0 GitHub release notes](https://github.com/givtaj/sleepapi/releases/tag/v0.1.0):
each new release must likewise record the full released commit SHA, a successful
CI run on that SHA, fresh tagged-source build results, and platform limitations.
The release tag and CI run must refer to the same commit. A local check or a workflow file alone is not release evidence.

The [alpha.1 release record](https://github.com/givtaj/sleepapi/releases/tag/v0.1.1-alpha.1)
contains its 39-test and installation evidence. Alpha.2 added three blocked-stdout
regressions, bringing the Rust suite to 42 tests; the
[alpha.3 release](https://github.com/givtaj/sleepapi/releases/tag/v0.1.1-alpha.3)
added 16 recovery-run tests for a total of 58. The alpha.4 fixture and transport
candidate has 86 Rust tests. Its merged implementation baseline
`535301ac7c4d2adcc6a8d33a6f121c71d5247d96` passed all four jobs in
[main CI run 40883400744](https://github.com/givtaj/sleepapi/actions/runs/40883400744).
The final release candidate still requires its own CI and tagged-source
installation checks; older results do not validate it. Windows host integration and native Windows builds
have not been tested.

The container smoke script checks health, scenario discovery returning seven
entries, timed failures on all four waiting routes, CORS, protocol headers,
recovery-run sequencing, receipts, generation guards, reset and deletion,
exact fixtures, connection drops and truncated responses, normal exit `0`,
and deadline exit `2` for an incomplete request. Rust tests
also cover preset definitions, validation, concurrency, cancellation, and parking
through early wakes. These checks are functional verification, not a throughput
benchmark or proof of production-scale capacity.

Use this checklist for each release and include the final results in its notes:

| Required check | Completion evidence |
| --- | --- |
| Release candidate committed and pushed | Full commit SHA and source link |
| Linux / Rust stable | Successful CI job including formatting, lint, Rust/Python tests, build, alpha smoke, and packaging |
| Linux / Rust 1.94.0 | Successful CI job including lint, tests, and build |
| Docker / linux/amd64 | Successful native build and container smoke checks |
| Docker / linux/arm64 | Successful native build and container smoke checks |
| Tag matches the tested commit | Annotated tag resolves to the same full SHA |
| Fresh public installation works | New tagged checkout, local Docker build, HTTP and shutdown checks |
| Source release is publicly available | GitHub release and source archive URLs work without authentication |

## Release sequence

1. **Prepare one release candidate.** Match `Cargo.toml` and the root package in
   `Cargo.lock` to `0.1.1-alpha.4`, and change the changelog's Unreleased heading
   to `0.1.1-alpha.4 — YYYY-MM-DD` using the actual release date. Update the README
   candidate wording and readiness status for publication; review features and limits. Check all README examples and commit the final files. A local
   Docker tag such as `sleepapi:dev` does not create a Git tag or publish an image.

2. **Push or merge the candidate to `main`, then require green CI.** Check
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

3. **Tag the exact green commit as `v0.1.1-alpha.4`.** Create an annotated Git tag and
   push that tag without moving it to a different commit later. Verify that
   `git rev-parse 'v0.1.1-alpha.4^{commit}'` equals the SHA recorded in step 2. If a
   published version needs a fix, prepare a new version instead of
   replacing the old tag.

4. **Verify the published source path before announcing the release.** In a
   fresh directory, use the README's version-pinned clone and Docker commands,
   substituting `v0.1.1-alpha.4` and image tag `sleepapi:0.1.1-alpha.4`.
   Confirm `GET /health` returns `{"status":"ok"}` and a POST simulation waits
   and returns its documented JSON. Run the existing container checks against
   that locally built image:

   ```sh
   python3 scripts/docker-smoke.py sleepapi:0.1.1-alpha.4
   ```

   This needs Docker and Python 3. The test creates and cleans up only its own
   temporary containers. Also check the GitHub source archive contains the
   Dockerfile, `Cargo.lock`, `src/`, and `LICENSE`; the build must not depend on
   an existing `target/` directory or local secrets.

5. **Publish a GitHub prerelease using the tested tag.** Use `v0.1.1-alpha.4` as
   the title, enable the prerelease flag, summarize the changes and limits, and link the
   README's Docker instructions. Describe the deliverable as source plus a
   Dockerfile. [GitHub provides source archives](https://docs.github.com/en/repositories/releasing-projects-on-github/about-releases);
   no prebuilt binary or container asset is promised. Verify the release and
   source links in a signed-out view.

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
cargo package --locked
docker build -t sleepapi:ci .
python3 scripts/docker-smoke.py sleepapi:ci
```

`cargo package` checks source packaging; it does not publish to crates.io. Run it
from a clean committed checkout. The full CI workflow additionally lints/builds
on Rust 1.94.0 and builds/tests the Dockerfile on both native Linux architectures.

## Publication record

This is a checklist template, not current publication status. Copy it into the
GitHub prerelease notes and fill every field from actual results:

- Candidate/tag: `v0.1.1-alpha.4`; full commit SHA: pending.
- Exact-commit `main` CI URL and all four required job results: pending.
- Alpha smoke JSON receipt in the stable CI log: pending.
- Annotated tag target equals the tested SHA: pending.
- Fresh tagged-source Docker build and smoke, platform, date: pending.
- Signed-out release and source-archive verification: pending.
- Distribution: source plus Dockerfile; Windows remains unverified.

A green PR run makes the candidate reviewable. After merging, require a green
`main` run for the resulting commit before tagging. Any subsequent source,
version, workflow, or documentation edit creates a new candidate. A published
release is immutable: corrections use a new version. If installation fails,
stop publication and use the previous `v0.1.1-alpha.3` source tag while preparing a fix.

# Releasing SleepAPI

## v0.1.0 scope

Ship a GitHub source release containing the Rust application, lockfile, Dockerfile,
tests, and documentation. Users build their own image. Linux AMD64 and ARM64
containers run through Docker on Linux, macOS, and Windows with Linux-container
support. A native Rust build requires Rust 1.94 or newer.

The first release does not need a container registry, image-publishing workflow,
native binary downloads, crates.io publication, or CI jobs for macOS and Windows.
Ordinary Linux CI builds and tests containers without publishing them.

## Release evidence

Keep the verification record in the [v0.1.0 GitHub release notes](https://github.com/givtaj/sleepapi/releases/tag/v0.1.0):
the full released commit SHA, a successful CI run on that SHA, fresh tagged-source
build results, and platform limitations. The release tag and CI run must refer to
the same commit. A local check or a workflow file alone is not release evidence.

The local implementation baseline, `25daf094fdfbc28d0695d95ac494b612d18e8f4a`,
passed 39 tests on Rust stable and 1.94.0, formatting, stable Clippy, source
packaging, and container smoke checks. The ARM64 container ran natively on the
local Mac; AMD64 used emulation. The GitHub jobs provide native Linux verification
for both container architectures. Windows host integration and native Windows
builds have not been tested.

The container smoke script checks health, scenario discovery returning seven
entries, timed failures on all four waiting routes, CORS, protocol headers,
normal exit `0`, and deadline exit `2` for an incomplete request. Rust tests
also cover preset definitions, validation, concurrency, cancellation, and parking
through early wakes. These checks are functional verification, not a throughput
benchmark or proof of production-scale capacity.

Use this checklist for each release and include the final results in its notes:

| Required check | Completion evidence |
| --- | --- |
| Release candidate committed and pushed | Full commit SHA and source link |
| Linux / Rust stable | Successful CI job including formatting, lint, tests, build, and packaging |
| Linux / Rust 1.94.0 | Successful CI job including lint, tests, and build |
| Docker / linux/amd64 | Successful native build and container smoke checks |
| Docker / linux/arm64 | Successful native build and container smoke checks |
| Tag matches the tested commit | Annotated tag resolves to the same full SHA |
| Fresh public installation works | New tagged checkout, local Docker build, HTTP and shutdown checks |
| Source release is publicly available | GitHub release and source archive URLs work without authentication |

## Release sequence

1. **Prepare one release candidate.** Keep `Cargo.toml` at `0.1.0`, ensure
   `Cargo.lock` is included, and change the changelog's planned heading to
   `0.1.0 — YYYY-MM-DD` using the actual release date. Review its feature list
   and limits. Check all README examples and commit the final files. A local
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

3. **Tag the exact green commit as `v0.1.0`.** Create an annotated Git tag and
   push that tag without moving it to a different commit later. Verify that
   `git rev-parse 'v0.1.0^{commit}'` equals the SHA recorded in step 2. If a
   published version needs a fix, prepare a new patch release instead of
   replacing the old tag.

4. **Verify the published source path before announcing the release.** In a
   fresh directory, use the README's version-pinned clone and Docker commands.
   Confirm `GET /health` returns `{"status":"ok"}` and a POST simulation waits
   and returns its documented JSON. Run the existing container checks against
   that locally built image:

   ```sh
   python3 scripts/docker-smoke.py sleepapi:0.1.0
   ```

   This needs Docker and Python 3. The test creates and cleans up only its own
   temporary containers. Also check the GitHub source archive contains the
   Dockerfile, `Cargo.lock`, `src/`, and `LICENSE`; the build must not depend on
   an existing `target/` directory or local secrets.

5. **Publish the GitHub release using the existing tag.** Use `v0.1.0` as the
   title, summarize the changelog's initial features and limits, and link the
   README's Docker instructions. Describe the deliverable as source plus a
   Dockerfile. [GitHub provides source archives](https://docs.github.com/en/repositories/releasing-projects-on-github/about-releases);
   no prebuilt binary or container asset is promised. Verify the release and
   source links in a signed-out view.

6. **Record completion.** Update the readiness record with the released commit,
   successful CI run, release URL, and fresh-install result. Keep verification
   notes distinct from feature changes. Creating a release is a publication
   step; updating these documents alone does not perform it.

## Local checks when implementation changes

Run the same checks used by CI before preparing the final release commit:

```sh
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
cargo +1.94.0 test --locked
cargo build --release --locked
cargo package --locked
docker build -t sleepapi:ci .
python3 scripts/docker-smoke.py sleepapi:ci
```

`cargo package` checks source packaging; it does not publish to crates.io. Run it
from a clean committed checkout. The full CI workflow additionally lints/builds
on Rust 1.94.0 and builds/tests the Dockerfile on both native Linux architectures.

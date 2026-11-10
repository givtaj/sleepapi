# Releasing SleepAPI

The current candidate is **`0.1.1-alpha.5`**, not yet released. Preparation stops
at a reviewed pull request with passing checks on its exact head. Merging,
tagging, and publication are separate maintainer decisions. In particular,
publishing a GitHub release automatically starts the container publication
workflow; it is not a source-only action.

The published [alpha.4 release](https://github.com/givtaj/sleepapi/releases/tag/v0.1.1-alpha.4)
and `ghcr.io/givtaj/sleepapi:0.1.1-alpha.4` remain the installation fallback and
README quickstart target. They do not include `/metadata`.

## Distribution scope

Ship a GitHub source release containing the Rust application, lockfile, Dockerfile,
tests, and documentation, plus a versioned image at `ghcr.io/givtaj/sleepapi`.
Users can pull the image or build it themselves. Linux AMD64 and ARM64
containers run through Docker on Linux, macOS, and Windows with Linux-container
support. A native Rust build requires Rust 1.94 or newer.

Ordinary Linux CI builds and tests containers without publishing them. The
separate [Container images workflow](../.github/workflows/container-images.yml)
publishes images for existing source releases. Native binary downloads,
crates.io publication, and native macOS/Windows CI remain outside this track.

## Publish versioned container images

After publication is approved, publishing a GitHub release runs the Container
images workflow when the tagged source includes that workflow. A manual
dispatch is available for an already published source tag whose image version
has not been published. Set the intended version first:

```sh
SLEEPAPI_RELEASE_VERSION=0.1.1-alpha.5
SLEEPAPI_RELEASE_TAG="v${SLEEPAPI_RELEASE_VERSION}"
gh workflow run container-images.yml --ref main -f tag="$SLEEPAPI_RELEASE_TAG"
```

These are future publication instructions, not candidate-preparation steps.
Do not dispatch a second publication for the existing alpha.4 image.

The workflow resolves the tag to one commit and requires successful `main` CI
on that source, including both Rust and both Docker jobs. It verifies the Cargo
version, builds on native AMD64/ARM64 runners, and runs `docker-smoke.py` before
pushing the exact tested images under unique staging tags. Only after both
architectures pass does it create `ghcr.io/givtaj/sleepapi:<version>` from
their registry digests. It then pulls anonymously on fresh native runners,
checks architecture/version/revision/digest, and reruns HTTP and shutdown smoke.
Pull requests exercise the build/test path without registry writes.

The workflow uses the repository's `GITHUB_TOKEN` with `packages: write` for
publishing; no personal registry secret is required. On the **first publication**,
GHCR creates a private package. Set the SleepAPI package visibility to **Public**
in its GitHub package settings. Promotion intentionally fails until both staging
images can be read anonymously. After changing visibility, rerun failed jobs;
keep the successful build jobs and their digest artifacts.

Version tags omit the source tag's leading `v`; no `latest` alias is published.
The workflow serializes publications and refuses to replace an existing version.
Authentication and network failures never count as proof that a tag is absent.
An interrupted promotion can resume without writing the tag again only when the
existing manifest references the exact two tested digests from that run.
This is a workflow guard, not registry-enforced immutability: retain the recorded
manifest digest for exact artifact pinning. A rebuilt image needs a new version.

If post-publication verification fails, investigate and rerun only the failed
verification jobs; do not overwrite the version. Save the workflow URL, source
SHA, manifest/platform digests from `container-release.json`, and both anonymous
verification results in the release notes. Staging tags use
`build-<run-id>-<attempt>-<architecture>` and are not installation instructions.

## Release evidence

The alpha.5 implementation baseline
[`a833eecfe6b7268378a3c3ae17b59d62ede2b115`](https://github.com/givtaj/sleepapi/commit/a833eecfe6b7268378a3c3ae17b59d62ede2b115)
passed [main CI run 42786430040](https://github.com/givtaj/sleepapi/actions/runs/42786430040).
It includes 96 Rust and 45 Python tests. That run validates the implementation
before the alpha.5 version and documentation changes; it does not validate this
candidate's final head. Record candidate checks in its PR, then require a new
`main` run on the resulting commit if a merge is later approved. Tagging, fresh
tagged-source installation, and publication remain separate
[alpha gates](ALPHA_READINESS.md).

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
whose `/metadata` route returns `404`. The container publication workflow's
smoke calls currently omit the flag; record an additional metadata-enabled
check against the downloaded alpha.5 artifact before announcing it.

Use this checklist for each release and include the final results in its notes:

| Required check | Completion evidence |
| --- | --- |
| Release candidate committed and pushed | Full commit SHA and source link |
| Linux / Rust stable | Successful CI job including formatting, lint, 96 Rust/45 Python tests, build, alpha smoke, retry-client example, and packaging |
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
   readiness, while retaining the currently published image in the quickstart.
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
   substituting the chosen source tag and local image tag
   `sleepapi:${SLEEPAPI_RELEASE_VERSION}`.
   Confirm `GET /health` returns `{"status":"ok"}` and a POST simulation waits
   and returns its documented JSON. Run the existing container checks against
   that locally built image:

   ```sh
   python3 scripts/docker-smoke.py "sleepapi:${SLEEPAPI_RELEASE_VERSION}" --metadata
   ```

   This needs Docker and Python 3. The test creates and cleans up only its own
   temporary containers. Also check the GitHub source archive contains the
   Dockerfile, `Cargo.lock`, `src/`, and `LICENSE`; the build must not depend on
   an existing `target/` directory or local secrets.

5. **Publish the approved GitHub prerelease using the tested tag.** Use the tag as
   the title, enable the prerelease flag, summarize the changes and limits, and link the
   README's Docker instructions. [GitHub provides source archives](https://docs.github.com/en/repositories/releasing-projects-on-github/about-releases).
   This triggers container publication automatically. Complete its native and
   anonymous verification, plus metadata smoke on the downloaded alpha.5 images,
   before announcing them or moving the quickstart to the new version. Verify
   the release and source links in a signed-out view.

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
- Container workflow URL, manifest/platform digests, source SHA: pending.
- Anonymous AMD64/ARM64 pulls and smoke results: pending.
- Metadata-enabled smoke and capability-client results on downloaded images: pending.
- Distribution: source plus Dockerfile and versioned container; Windows remains unverified.

A green PR run makes the candidate reviewable. After merging, require a green
`main` run for the resulting commit before tagging. Any subsequent source,
version, workflow, or documentation edit creates a new candidate. A published
release is immutable: corrections use a new version. If installation fails,
stop publication and use the published `v0.1.1-alpha.4` source tag or
`ghcr.io/givtaj/sleepapi:0.1.1-alpha.4` image while preparing a new version.

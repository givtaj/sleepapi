# Alpha.6 source release verification — 25 November 2026

**PASS:** [v0.1.1-alpha.6](https://github.com/givtaj/sleepapi/releases/tag/v0.1.1-alpha.6)
was published as a source prerelease on **25 November 2026 at 11:20:35
Europe/Madrid (10:20:35 UTC)**. It combines the four fixes from PR #26 with the
lost-final-reply change from PR #27; see the [five changes](../../../CHANGELOG.md#011-alpha6--2026-11-25).
Distribution is source plus a Dockerfile. No Docker images or executable assets
were published.

| Release identity | Value |
| --- | --- |
| Tag | `v0.1.1-alpha.6` |
| Annotated tag object | `e66f91429a0c89d92d89ad0ba50062e981ee3096` |
| Released commit | [`0085226a91eb5f5a989de7377bb0e3e35a21d439`](https://github.com/givtaj/sleepapi/commit/0085226a91eb5f5a989de7377bb0e3e35a21d439) |
| Source tree | `6c51af48cfa62b8b61ae46693d7ecbd222d083df` |
| Exact-commit main CI | [45592959784](https://github.com/givtaj/sleepapi/actions/runs/45592959784) |
| [Source TAR](https://github.com/givtaj/sleepapi/archive/refs/tags/v0.1.1-alpha.6.tar.gz) SHA-256 | `d03d2e395faff6606a5e681549f35500dea10818a9fc09c9adc5fa5dfc6fc822` |
| [Source ZIP](https://github.com/givtaj/sleepapi/archive/refs/tags/v0.1.1-alpha.6.zip) SHA-256 | `c6ce902c3d1069c0e8cbf35314240cbfa6c8605d56ad1f338b4acf6d92025581` |

Both source archives matched all **565 tracked files** at the released commit.
The fresh tagged checkout had no `target/` directory before building. Earlier
tags were unchanged.

| Verification | Result |
| --- | --- |
| Linux Rust stable and Rust 1.94.0 | PASS: 100 Rust tests on each toolchain; 43 Python tests on stable; formatting, lint, builds, alpha smoke, retry example, packaging and stable installed-package controls passed |
| Native Linux Docker AMD64 and ARM64 CI | PASS: both builds and container smoke jobs on the exact released commit |
| Fresh local Docker installation | PASS: `--pull --no-cache`, no platform override, default Linux ARM64 on macOS ARM64; non-root UID 65532; metadata, HTTP/CORS, fixtures, transport faults, recovery/reset/replay and shutdown |
| Fresh native installation | PASS: `cargo install --locked --path` into fresh install and target directories; macOS ARM64, Rust/Cargo 1.99.1, Python 3.9.6 |
| Restart after `connection_drop` | PASS: zero initial response bytes; receipt `provider_accepted: false`; distinct restarted worker retries once, leaving two attempt receipts through three polls |
| Restart after `accept_then_drop` | PASS: zero initial response bytes; receipt `provider_accepted: true`; distinct restarted worker sends nothing further through three polls, leaving one receipt and the next success step unused |
| Owned-resource cleanup | PASS: both native runs `deleted_and_absent`, native server exit `0`; Docker owned container removed, shutdown exit `0`, no cleanup errors |
| Anonymous public access | PASS: release page/API returned `200`; TAR/ZIP redirects ended at `200`; prerelease flag true, draft false, zero uploaded assets; final release-note readback matched |

The native controls used direct HTTP/1.1 without `Expect: 100-continue`, correlated
the original run/generation/attempt, and restarted the synthetic worker while
keeping SleepAPI running. Receipts describe simulated acceptance, not real
provider completion. They remain bounded and process-local; missing or evicted
receipts leave acceptance unknown.

| Locally built artifact | SHA-256 |
| --- | --- |
| Linux ARM64 Docker runtime binary | `9e18d40e7d62e8653627b18e18393e0cc870c3211a1d90e6f4bc29d9473a7f71` |
| Native macOS ARM64 installed binary | `baea586e0c0657ac58f7b4d2b256c1af79960855d7f23836a0166cee57d6dc00` |
| Local Docker image ID | `1f13b993e8425b7aa7cde9c98186188247008c46e65d78434b15015113f970a9` |

These hashes identify the local verification artifacts, not downloadable binary
releases. The host was macOS 26.0.1 ARM64 with Docker Engine 29.8.1. Cargo source
and registry caches could be reused, but installation and build-output directories
were fresh. Local AMD64, Windows, and customer acceptance were **NOT RUN**; CI
separately covered native Linux AMD64 and ARM64 containers. These functional
checks do not establish performance, production capacity, or customer acceptance.

The immutable alpha.6 tag retains preparation-time candidate wording and the
alpha.5 quickstart pin. This evidence and the updated alpha.6 quickstart are
completion documentation added later on main; they do not move or rewrite the
release tag. Alpha.5 and earlier evidence remains separate and unchanged.

Raw receipts: [installation, identity, counts and public access](tagged-source-install.json),
[exact-commit CI](release-ci.json), [source file manifest](source-manifest.json),
[native lost-reply controls](lost-reply.json), [retry/reset/replay](retry-client.json),
[Docker build](docker-build.log), [Docker smoke](docker-smoke.log), and
[native installation](native-install.log). The installation receipt records
SHA-256 digests for the other retained files.

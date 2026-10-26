# Local alpha preparation evidence — 2026-10-26

Candidate package: `0.1.1-alpha.1`, prepared from main
`6f282c5b2147240d8996bc879898d7d92ee042ff` plus the preparation diff.
Environment: macOS 26.0.1 ARM64, stable Rust 1.98.1, minimum Rust 1.94.0,
Python 3.9.6. The local Docker daemon was stopped.

Commands completed before the receipt was captured:

```sh
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
cargo +1.94.0 test --locked
cargo +1.94.0 build --release --locked
cargo build --release --locked
python3 -B -m unittest discover -s scripts -p 'test_*.py' -v
python3 scripts/alpha-smoke.py --binary target/release/sleepapi \
  --output docs/release-evidence/2026-10-26/alpha-smoke-macos-arm64.json
```

All 39 Rust tests passed on each toolchain and all six Python tests passed.
The stable release build immediately preceded the HTTP smoke. The JSON receipt
records binary and harness hashes, the source snapshot, workload, thresholds,
and per-route results. Its source digest can be compared with the committed
candidate, but hashes alone are not proof of how an arbitrary binary was built.
The CI sequence rebuilds the binary from its checkout before running the gate.

The subsequent `cargo package --locked --allow-dirty` verified a 75-file source
snapshot. Final CI requires clean committed packaging. Minimum-toolchain Clippy
was unavailable locally and remains a required CI check. Local socket access was
needed for the process tests and HTTP smoke; their initial sandbox permission
errors were environmental and were resolved by rerunning with socket access.

This receipt verifies the specified local regression workload. It is not Docker,
Windows, production-capacity, published-tag, or fresh-install evidence. The
preparation PR and final release notes own those later verification results.

# Lost-reply verification on 23 November 2026

Both candidate faults selected `200` and closed the worker's connection with
**zero HTTP response bytes**. After a worker restart, their receipts led to
different decisions. A second success step stayed available throughout each
test so an extra provider call would consume a step and leave a receipt.

| Server and first step | What the first receipt recorded | Worker sends across three polls after restart | Final admitted attempts |
| --- | --- | ---: | ---: |
| Released alpha.5, `connection_drop` | `fault_injected`, `before_headers`, zero injected body bytes. Acceptance field absent. | 1, allowed by the synthetic control's retry policy | 2 |
| Released alpha.5, complete `200` swallowed by the test proxy | SleepAPI recorded `completed`. The proxy read the full success fixture, recorded acceptance separately and forwarded zero bytes. | 0 | 1 |
| Candidate, `connection_drop` | `fault_injected`, `before_headers`, zero injected body bytes, `provider_accepted: false` | 1 | 2 |
| Candidate, `accept_then_drop` | `fault_injected`, `before_headers`, zero injected body bytes, `provider_accepted: true` | 0 | 1 |

Alpha.5 rejected the native `accept_then_drop` configuration with HTTP `422`.
Each comparison case used a fresh run and two worker processes. The first
worker persisted a synthetic intent and measured the raw response through EOF
or reset. The restarted worker loaded the ambiguous intent and read the receipt
for the same run, generation, and attempt. The harness checked its decision
against the provider's receipts before and after all three polls.

The full [comparison receipt](comparison.json) includes the wire observations,
both receipt snapshots, worker identities and decisions, and cleanup results.
The [build and regression record](verification.json) binds the receipt to the
exact binaries and source. The tested simulator revision is
`788042f45d1bb1a2f21bd0fca1164088c1f7956f`, based on
`71b4283905352628f5654631f640ea228049edb5`. The public alpha.5 tag resolves to
`f10bd4c9551f4617f68e9ba7aa08385e178dff6a`. Both binaries declare package
version `0.1.1-alpha.5`.

## Reproduce locally

From a checkout or unpacked source archive containing this feature, use an unused
temporary directory. These commands download and verify the alpha.5 source,
build it and the source in the current directory, then run the
[worker harness](../../../scripts/verify-lost-reply.py). The historical receipt
used the harness at `e65f1ba9f7ea4a06a89c03dbee0b09e1967df821`, whose server
source matches `788042f`. A rerun records the hashes of the binaries supplied.

```sh
set -e
work=$(mktemp -d)
mkdir -p "$work/alpha5"
curl -fsSL https://github.com/givtaj/sleepapi/archive/refs/tags/v0.1.1-alpha.5.tar.gz -o "$work/alpha5.tar.gz"
(cd "$work" && echo '65b9200f08127ba609c9ae2cbaa5a8c81aef0668c36509015c4420c03635e54f  alpha5.tar.gz' | shasum -a 256 -c -)
tar -xzf "$work/alpha5.tar.gz" --strip-components=1 -C "$work/alpha5"
cargo install --locked --path "$work/alpha5" --root "$work/install" --target-dir "$work/alpha5-target"
cargo build --release --locked --target-dir "$work/candidate-target"
python3 -B scripts/verify-lost-reply.py \
  --alpha5-binary "$work/install/bin/sleepapi" \
  --candidate-binary "$work/candidate-target/release/sleepapi" > "$work/comparison.json"
```

The harness exits zero on success and emits JSON. Each worker used a direct
HTTP/1.1 request without `Expect: 100-continue`. The proxy case had one local
intermediary. SleepAPI stayed running while the worker restarted. Receipts are
bounded, process-local state and disappear if SleepAPI restarts. Verification
covered native macOS ARM64 and three worker polls per case. The candidate is
unreleased.

# Retry-client acceptance example

[`examples/retry-client.py`](../examples/retry-client.py) is a small consumer of
the alpha.4 HTTP API. It uses Python's standard library to exercise finite retry
flows and writes a JSON result with client observations and separate server
receipts. Use Python 3.9 or newer. No Python packages are required.

## Run against the pinned release

The example is a post-alpha.4 addition: keep it in your current checkout and
build the server from the immutable `v0.1.1-alpha.4` tag in a separate directory.
From the checkout containing this example:

```sh
git clone --branch v0.1.1-alpha.4 --depth 1 https://github.com/givtaj/sleepapi.git /tmp/sleepapi-client-alpha4
cargo build --release --locked --manifest-path /tmp/sleepapi-client-alpha4/Cargo.toml
python3 examples/retry-client.py \
  --binary /tmp/sleepapi-client-alpha4/target/release/sleepapi \
  --output /tmp/sleepapi-client-acceptance.json
```

Choose unused checkout and output paths. Native builds require Rust 1.94 or
newer. `--binary` launches an owned server on an ephemeral loopback port, drains
its logs, and stops it when the example finishes. The result records the binary's
SHA-256. The API does not expose a server version, so the target release label
identifies the intended contract, not a version detected over HTTP.

Alternatively, start the [tagged Docker build](../README.md#run-with-docker) and
run the example from your current checkout:

```sh
python3 examples/retry-client.py --base-url http://127.0.0.1:3000
```

The existing-server mode creates and deletes only its own runs. It does not stop
the server. Use an otherwise idle local instance so other work cannot exhaust
shared capacity during the demonstration. Requests use HTTP/1.1; a proxy that
buffers or rewrites deliberately broken responses changes what the client sees.

For development against your current source:

```sh
cargo build --release --locked
python3 examples/retry-client.py --binary target/release/sleepapi
```

CI runs this command against the binary it builds. That checks the current
source, independently of the tagged-release check above.

## What passes

- A three-attempt `503 → 503 → 200` sequence delivers exact fixture bytes,
  content types, and configured headers. The client applies a finite retry
  policy instead of asking the server to retry for it.
- A second sequence closes before response headers, then returns an incomplete
  fixture prefix, then succeeds. The client distinguishes a missing response
  from a truncated body and checks the exact bytes received.
- Reset advances the generation, clears receipts, and repeats each sequence.
  A stale generation is rejected without consuming a step.
- Separate receipt requests correlate each attempt with its run, generation,
  sequence position, and server outcome. Cleanup removes the example's runs.

Unexpected responses, failed assertions, or failed cleanup produce a nonzero
exit. Successful output is JSON suitable for retaining with the tested source,
platform, and build method. It does not include a claim that the original
blocked-stdout reproduction passed; that has separate process regressions.

## Applying this to your client

The successful example proves how this Python client observed these scenarios.
For external acceptance, run the same sequences through the consuming app's
HTTP library and retry policy, then record its pass/fail results, platform, build
method, and any reproduction. A server receipt saying `fault_injected` does not
prove delivery, and `handler_cancelled` does not prove blocking work stopped.
Keep client observations alongside, rather than substituting, those receipts.

The example's retries are appropriate for these synthetic run attempts. Retrying
an arbitrary application POST needs its own idempotency policy. The supported
fixture, transport, capacity, and lifetime limits remain those in the
[fixture/fault guide](response-fixtures.md) and [recovery-run guide](recovery-runs.md).

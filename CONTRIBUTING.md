# Contributing to SleepAPI

Contributions are welcome: clearer documentation, reproducible bug reports,
client examples, and focused fixes help make SleepAPI easier to use.

SleepAPI is a local tool for testing HTTP delays, failures, and recovery. Start
with the [quickstart](README.md#quickstart-two-failures-then-success) to try it.

## Before you start

Search the [existing issues](https://github.com/givtaj/sleepapi/issues) for related
work. Small fixes and documentation improvements can go straight to a pull
request. For larger features or API changes, open a feature request describing
the problem and intended behavior first so the scope can be discussed.

## Work on a change

Fork the repository, clone your fork, and create a branch from current `main`.
Install Rust 1.94 or newer and Python 3 for the development checks. Docker is
needed for container checks; `curl` and `jq` are used by the README quickstart.

From your checkout, start the server with:

```sh
cargo run --locked
```

It listens on `http://127.0.0.1:3000` by default. Stop it with Ctrl-C.

For code changes, run:

```sh
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
python3 -B -m unittest discover -s scripts -p 'test_*.py' -v
```

For HTTP or lifecycle changes, also run the smoke and retry-client checks in
[Develop](README.md#develop). That section includes Docker checks and explains
the CI coverage. Add or update regression coverage for behavior changes, and
update the relevant API documentation when the contract changes.

For documentation-only changes, check links and any commands or examples you
change. You do not need to run unrelated benchmarks. Report the checks you ran
and anything you could not verify; GitHub Actions runs the repository's CI.

## Open a pull request

- Target `main` and keep the change focused on one problem.
- Describe the problem, resulting behavior, and how you verified the change.
- Link a related issue when there is one; an issue is not required for small fixes.
- Address review feedback and make sure CI passes before merging.

## Report a bug or request a feature

Use the [issue templates](https://github.com/givtaj/sleepapi/issues/new/choose).
For bugs, include the release tag or commit, your environment, and the smallest
reproduction you can share. For retry, timeout, or transport faults, include what
the client observed as well as any server receipts; they describe different
parts of the request.

Remove credentials, private URLs, personal information, and customer data from
public reports, logs, and example payloads. Keep discussions respectful and
constructive.

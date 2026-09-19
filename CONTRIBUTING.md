# Contributing to Ting

Small fixes, examples, bug reports, and thoughtful critiques are welcome. You do
not need accounts with model providers to run the automated tests.

## Get set up

Ting currently targets macOS and Linux. Participant commands use a Unix shell;
native Windows support is not currently tested. Windows users can try WSL.

- Rust 1.88 or newer, installed with [rustup](https://rustup.rs/).
- Rust 1.92.0 with `rustfmt` and `clippy` for the same style checks as CI.
- Node.js 22 or newer for dependency-free browser reducer tests.
- Go 1.24 or newer only when working on the optional TUI.

```sh
git clone https://github.com/Abeansits/ting.git
cd ting
rustup toolchain install 1.92.0 --profile minimal --component rustfmt --component clippy
cargo build --locked
cargo run -- --help
```

## Check your change

```sh
cargo +1.92.0 fmt --all -- --check
cargo +1.92.0 clippy --locked --all-targets -- -D warnings
cargo test --locked
node --test tests/dashboard.test.cjs
```

For a TUI change, also run these from `tui/`:

```sh
gofmt -w .
go vet ./...
go test -race ./...
go build ./...
```

CI tests Rust on Linux with the minimum compiler and current stable, and on macOS
with stable. Go tests run with the race detector on both operating systems.
Changes to files shared with the dashboard or TUI should exercise both consumers.

The Rust tests use local fake model commands. A live model run is a separate,
optional check: it can send context to providers and consume credits. State
clearly whether you tested with fake commands, real providers, or both.

## Where to start

- [`ROADMAP.md`](ROADMAP.md): planned milestones and completed PRs.
- [`docs/PROJECT-REVIEW.md`](docs/PROJECT-REVIEW.md): actionable findings and acceptance criteria.
- `src/main.rs`: CLI commands.
- `src/protocol.rs`: rounds, participant invocation, and final output.
- `src/substrate.rs`: files, process execution, and response collection.
- `src/convergence.rs`, `src/synthesis.rs`: evaluation and generation prompts.
- `src/events.rs`, `src/run_status.rs`, `schemas/`: persisted state and events.
- `src/server.rs`, `src/static/`: browser dashboard and event transport.
- `tui/`: independent Go client for the same filesystem contract.

A focused regression test, clearer error message, or missing example is a good
first contribution. For a larger change, open an issue describing the problem
and expected behavior before building it.

## Pull requests

Create a branch and keep each PR focused. Explain the trigger, the behavior after
the change, and the checks you ran. Include screenshots for visible UI changes.
Update user-facing docs or the changelog when behavior changes. Preserve dissent
and original participant evidence; agreement is not proof of correctness.

CodeRabbit feedback can be useful, but it is advisory. Automated tests and
maintainer review determine whether a change is ready; a skipped bot review is
not evidence that the code was reviewed.

Be kind and specific. Discuss the work, explain tradeoffs, and assume contributors
are here to help. Maintainers may remove harassment, spam, or personal attacks.

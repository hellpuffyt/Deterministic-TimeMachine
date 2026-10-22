# Contributing to Deterministic-TimeMachine

Thanks for your interest! The project's goal is to make flaky tests reproducible **without overclaiming**: everything it says it controls must really be controlled, and everything else must be documented. Contributions that improve correctness, diagnostics or documentation are welcome.

## Getting started

```sh
git clone https://github.com/hellpuffyt/Deterministic-TimeMachine && cd Deterministic-TimeMachine
cargo test
```

Look at [docs/good-first-issues.md](docs/good-first-issues.md) for starter tasks, and [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for how the pieces fit.

## Before opening a pull request

```sh
cargo fmt --all
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features
```

## Ground rules

- **New inputs go through `Ctx::interact`.** Add a `Kind`, a method in `src/ctx/inputs.rs`, and a record → replay test (plus a `Forbidden`-source assertion in `tests/record_replay.rs` if replay could otherwise touch the outside world).
- **Bug fixes come with a regression test** that fails before the fix.
- **Trace format changes bump `trace::VERSION`** and keep reader errors actionable. Regenerate committed fixtures only when the input sequence intentionally changed, and say why in the PR.
- **Do not change expected values in tests** (golden RNG values, fixtures, failure rates) without a written justification.
- **Be precise in docs.** If something is untested on a platform, say so. Numbers in docs must come from a command you ran, with the machine and conditions stated.
- **No secrets in traces or tests.** Do not commit traces recorded with real sources without reviewing them.

## Reporting bugs

Use the bug report template. A `.tmtrace` file plus the smallest program that replays it is the ideal reproducer — this project should be good at that.

## Code of conduct

This project follows the [Contributor Covenant](CODE_OF_CONDUCT.md).

## License

By contributing you agree that your contributions are dual-licensed under MIT and Apache-2.0, as described in the README.

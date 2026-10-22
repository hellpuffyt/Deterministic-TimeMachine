## What and why

<!-- One logical change. Link the issue if there is one. -->

## Checklist

- [ ] `cargo fmt --all` and `cargo clippy --all-targets --all-features -- -D warnings` are clean
- [ ] `cargo test --all-features` passes
- [ ] Changes to the trace format bump `trace::VERSION` and keep the reader's error messages actionable
- [ ] New nondeterminism sources are recorded through `Ctx::interact` and covered by a record → replay test
- [ ] Bug fixes come with a regression test
- [ ] Committed `.tmtrace` fixtures were regenerated only when the event stream intentionally changed (explained above)

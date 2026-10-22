# Known limitations

- **Only `Ctx` inputs are controlled.** See [UNSUPPORTED.md](UNSUPPORTED.md) for the full list of what is not.
- **No async adapter yet.** `ctx.sleep` blocks (real clock) or advances virtual time (simulated clock); there is no `Future`-returning timer. Async code can still call `ctx.now()`/`next_u64()` etc., but async sleeps and timeouts are outside the boundary.
- **One `Ctx`, one deterministic call order.** A `Ctx` can be shared across threads (it is `Send + Sync`), but replay only works if the calls happen in the same order. Use one thread or `sched`.
- **Traces are held fully in memory** while recording, after loading and during replay (about 150 bytes per small event; measured numbers in [DETERMINISM.md](DETERMINISM.md#memory-and-size)). Loading streams the file and refuses files over 1 GiB by default (`TraceError::TooLarge`; raise with `Trace::load_with_limit` or `TM_MAX_TRACE_BYTES`). Recording has no event cap and does not stream to disk.
- **The checksum covers event lines only.** Edits to the header or to the end line's outcome/message still parse; replay then reports "not reproduced".
- **Traces may contain sensitive data** read by the program under test (files, environment variables, response bodies). Nothing is redacted.
- **The checksum is not tamper-proof.** It detects accidents and casual edits, not a deliberate forgery; a trace is trusted input to the program you replay it into.
- **Fixing a bug usually changes the input sequence**, so replaying the *old* failing trace against the *fixed* code diverges rather than passes. Check fixes by re-running the same seeds with simulated sources (`tm hunt --sources sim --first-seed N --runs 1`, or `harness::explore`).
- **Minimization needs deterministic observations.** With real sources, non-decision inputs (time, files, network) can differ between runs; `minimize` detects this up front and refuses.
- **`tm` passes `TM_*` variables to the child only.** Programs that spawn further processes must forward them themselves.
- **Windows** is configured in CI but has not been observed passing yet (see [PRODUCTION_READINESS.md](PRODUCTION_READINESS.md)).
- **Not on crates.io.** Use the git dependency shown in the README.

## Roadmap

Achievable next steps, roughly in order:

1. Async adapter: virtual-time timers usable from async code (behind a feature flag), with tests on a single-threaded executor.
2. `#[timemachine::test]` attribute macro that wires `from_env` into ordinary `#[test]` functions and saves failing traces automatically.
3. Redaction hooks for recorded values; streaming the trace to disk while recording.
4. `tm hunt --jobs N` to run seeds in parallel.
5. Publish 0.1 to crates.io once the API has had outside feedback.

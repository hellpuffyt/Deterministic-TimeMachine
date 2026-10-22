# Changelog

All notable changes are documented here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project uses [Semantic Versioning](https://semver.org/) (0.x: breaking changes may happen in minor versions).

## [Unreleased]

### Added
- `docs/DETERMINISM.md`: the precise replay guarantee, what is and is not controlled, trace-integrity checks, and measured memory for 1M/10M-event traces.
- `Trace::load_with_limit`, `Trace::load_with`, `trace::DEFAULT_MAX_TRACE_BYTES` (1 GiB), `TraceError::TooLarge`, and the `TM_MAX_TRACE_BYTES` variable (honoured by `tm` and `Builder::from_env`).
- `harness::replay_owned` (replay without copying the trace).
- `examples/trace_scale.rs` for measuring memory on large recordings.

### Fixed
- Loading a trace read the whole file into memory with no limit; `tm validate /dev/zero` grew without bound. Loading now streams line by line through a bounded reader.
- Saving built the whole trace text in memory before writing; it now streams, fsyncs, then renames, and removes the partial file on failure.
- `run_main` loaded the replayed trace a second time to learn the recorded outcome (twice the memory, and a window in which a replaced file could be compared against the wrong trace); it now uses the trace being replayed.
- Invalid UTF-8 in a trace file is reported as a malformed line with its line number instead of a generic I/O error.

## [0.1.0] - unreleased

### Added
- `Ctx` with live, record, replay and drive modes; inputs for time, randomness, choices, environment variables, files, network requests and checkpoints.
- Simulated sources (virtual clock, seeded xoshiro256\*\*, in-memory filesystem, map environment, closure network) and real sources (system clock, entropy RNG, confined real filesystem, process environment).
- Trace format v1 with event count and checksum; specific errors for truncation, reordering, edits, wrong format and wrong version.
- Divergence diagnostics with expected vs actual request, call sites, context and reason; configurable call-site checking.
- `harness::{run, explore, replay}`, `minimize::{ddmin, shrink_decisions, minimize}`, `sched::run`.
- `envmode` (`TM_*` variables, `Builder::from_env`, `run_main`, JSON run reports).
- `tm` CLI: `hunt`, `replay`, `minimize`, `inspect`, `diff`, `validate`.
- Examples `flaky_cache` (timing flakiness) and `lost_update` (ordering bug).

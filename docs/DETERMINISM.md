# What "deterministic replay" means here

This page states exactly what Deterministic-TimeMachine guarantees, under which conditions, and what it does not. Each statement names the code that implements it and the test that checks it. It is not universal replay: only inputs read through a `Ctx` are controlled.

## The guarantee

Let a *run* be one execution of a program `P` that reads its nondeterministic inputs through one `Ctx`. A run in record mode produces a trace `T = (e0, e1, …, en-1)` where each event `ei` is `(kind, key, value, site)`.

**Replay.** Replaying `T` against `P` answers the i-th `Ctx` request with `value(ei)` and never consults a source (clock, RNG, filesystem, environment, network). If `P` is a deterministic function of the values it receives through the `Ctx`, the replayed run issues the same requests in the same order, receives the same values, and therefore computes the same results and ends with the same outcome (passed / failed with the same message / panicked with the same message).

**Divergence instead of silent difference.** If the replayed run issues a request that differs from the recorded one — different kind or key, a different source file (default `SiteCheck::File`), a request past the end of the trace, or it finishes before consuming all events — replay stops at that request and reports a `Divergence` (index, expected vs actual request, three preceding events, reason, hint). It never substitutes a value of the wrong type or a value for a different request.

**Re-execution of decisions.** With simulated sources, a run is a pure function of the seed; and driving a fresh run with the recorded decisions (`rng.u64`, `choose`) reproduces the original event stream value for value. The minimizer relies on this.

| statement | implemented in | checked by |
|---|---|---|
| replay answers from the trace and never calls a source | `ctx::engine::Engine::Replay`, `Ctx::interact` | `tests/record_replay.rs::replay_never_consults_sources` |
| same inputs → same observable output | as above | `tests/record_replay.rs::replay_reproduces_observable_output` (property, 64 seeds/run) |
| simulated sources are a function of the seed | `sources::{VirtualClock, SeededRng, MemFs, MapEnv}` | `same_seed_same_trace` (property) |
| driving recorded decisions re-creates the run | `Engine::Drive` | `driving_the_recorded_decisions_recreates_the_original_run` (property) |
| checkpoints (`ctx.mark`) are reached in the recorded order with the recorded state | `Kind::Mark` events | `tests/divergence.rs::checkpoints_restore_the_recorded_state_in_order`, `reordered_checkpoints_diverge_at_the_first_out_of_order_mark` |
| every kind of mismatch is reported, not misread | `Ctx::interact`, `Ctx::finish`, `Ctx::bad_value` | `tests/divergence.rs` (13 tests) |
| outcome comparison: same outcome and message, or "not reproduced" | `harness::replay_owned`, `envmode::run_main` | `a_different_failure_is_not_reported_as_reproduced` |

## Inputs that are controlled

| input | `Ctx` methods | record | replay |
|---|---|---|---|
| wall-clock time | `now`, `now_nanos` | value from the clock source (virtual clock starts at 2025-01-01T00:00:00Z) | recorded value |
| sleeping | `sleep` | virtual clock: advances time, no wait; system clock: really sleeps | no wait; the requested duration is the event key and must match |
| randomness | `next_u64`, `gen_range`, `gen_bool`, `choose`, `shuffle` | seeded xoshiro256\*\* (implemented in-crate so a seed's stream never changes with a dependency upgrade) or OS-seeded entropy | recorded value |
| scheduling of *your* steps | `sched::run` (a `choose` per step) | recorded choice | recorded choice |
| environment variables | `env_var` | map or process environment | recorded value |
| files | `read`, `read_to_string`, `write`, `exists`, `remove`, `list` | in-memory fs or `RealFs` confined to a root | recorded value or recorded error; the real filesystem is never touched |
| network | `request` | closure-backed stub, or refused by default | recorded response or error |
| checkpoints | `mark` | label recorded | label must match |

## Inputs that are not controlled

Everything not read through a `Ctx`: OS thread scheduling, `Instant`/`SystemTime` called directly, timers inside other crates (including async runtimes — there is no async adapter), `HashMap` iteration order with `RandomState`, global RNGs, direct syscalls (`std::fs`, `std::env`, sockets, processes, signals), memory addresses, floating-point differences across platforms, data races and global state that survives between runs. [UNSUPPORTED.md](UNSUPPORTED.md) has the full table with workarounds.

If one of these changes which `Ctx` requests are made, replay reports a divergence. If it changes only the program's result, replay reports "did not reproduce". If it changes neither, it goes unnoticed — the tool cannot see it.

## Conditions the guarantee depends on

- **One deterministic call order.** A `Ctx` is `Send + Sync`, but replay matches requests by position. Requests from several OS threads in a racy order diverge (reported, not silent). Use one thread or `sched`.
- **The program, not just the trace, must be the same.** A code change that alters the request sequence diverges by design. Line moves within a file are tolerated by default (`SiteCheck::File`); `SiteCheck::Exact` and `Off` are available.
- **Unsupported runtimes and modes.** No async/virtual-time timers for tokio-style executors; no `#[test]` macro; programs that spawn child processes must forward `TM_*` themselves; Windows is verified only by the CI test suite.

## Trace integrity

What `Trace::load` / `Trace::parse` check, and what they do not:

| check | error |
|---|---|
| empty file | `Empty` |
| first line not a header / another tool's format / other version | `MissingHeader`, `WrongFormat`, `UnsupportedVersion` |
| a line that is not valid JSON of the expected shape, or not UTF-8 | `Malformed { line }` |
| event `seq` not 0, 1, 2, … (reordered, removed, duplicated events) | `BadSequence { line }` |
| missing end line (crash, kill, truncation at any byte) | `Truncated`, or `Malformed` for a cut inside a line |
| data after the end line, a second header | `TrailingData`, `DuplicateHeader` |
| end line's event count ≠ events present | `CountMismatch` |
| any byte of an event line changed | `ChecksumMismatch` (FNV-1a 64, not tamper-proof) |
| file larger than the size limit (default 1 GiB) | `TooLarge` |

**Not covered by the checksum:** the header line and the end line's `outcome`/`message`. An edit there still parses; replay then compares the program's real outcome with the edited one and reports "not reproduced" (`tests/trace_format.rs::header_and_footer_edits_are_not_covered_by_the_checksum_but_fail_replay`). CRLF line endings (from `core.autocrlf`) are accepted and do not affect the checksum.

## Memory and size

A trace is held in memory as a `Vec<Event>` while recording, after loading and during replay. Loading streams the file line by line (no copy of the file text), saving streams to disk, and `harness::replay_owned` replays without copying the trace. Memory is therefore linear in the number of events — about 150 bytes per small event, measured below — and loading is capped at `trace::DEFAULT_MAX_TRACE_BYTES` (1 GiB) unless the caller raises it (`Trace::load_with_limit`, or `TM_MAX_TRACE_BYTES` for `tm` and environment-driven programs). The cap also bounds endless inputs such as `/dev/zero` or a FIFO.

Peak resident memory, measured with `/usr/bin/time -l` and `examples/trace_scale.rs` (release build, Apple M5, macOS; one third each `rng.u64`, `clock.now`, `clock.sleep`; ~117 bytes per event on disk):

| events | file size | phase | before (0.1.0 as first committed) | after |
|---|---|---|---|---|
| 1,000,000 | 117 MB | record + save | 287 MB | 170 MB |
| 1,000,000 | | load + validate | 271 MB | 154 MB |
| 1,000,000 | | load + replay | 308 MB | 154 MB |
| 10,000,000 | 1.18 GB | record + save | 1,658 MB | 1,529 MB |
| 10,000,000 | | load + validate | 1,962 MB | 1,460 MB |
| 10,000,000 | | load + replay | 2,267 MB | 1,497 MB |
| endless (`/dev/zero`) | — | `tm validate` | grew without bound (1.5 GB after 3 s, never finished) | stops at the limit: `TooLarge` (1.08 GB peak at the 1 GiB default; 3.7 MB with `TM_MAX_TRACE_BYTES=4096`) |

Single runs, not a benchmark suite; reproduce with the commands at the top of `examples/trace_scale.rs`. The 10M-event file is above the default limit; the example raises it explicitly.

Recording has no event cap: a program that reads more inputs records more events, and the in-memory trace grows with it until the program ends. Streaming the trace to disk while recording is not implemented.

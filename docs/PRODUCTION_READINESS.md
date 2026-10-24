# Production readiness record

**Classification: Partially Verified** — everything in the acceptance checklist below passed on macOS (aarch64) from a clean clone, and the GitHub Actions workflow has since passed on Linux, macOS and Windows (GitHub Actions run 40115734341 on `7b1d772`, 2026-10-24); Linux and Windows are verified only by that CI run. The project describes itself as experimental 0.1.0; this record is evidence, not an endorsement.

| | |
|---|---|
| Tested commit | `7279441` (clean clone, 2026-10-22T05:13Z); an earlier clean-clone run of `0f4cd12` gave the same results. Later commits change only `docs/` |
| Date | 2026-10-22 |
| Machine | Apple M5, 10 cores, 16 GB, macOS 27.0.1; load average 8–56 during the runs (other builds were running) |
| Toolchains | rustc 1.98.0 (stable), rustc 1.85.1 (MSRV check) |

## Scope

A Rust library (`timemachine`) and CLI (`tm`) that make flaky tests reproducible by routing time, randomness, files, environment variables, network responses and scheduling choices through an injected `Ctx`, recording them to a versioned trace, replaying them with divergence diagnostics, and minimizing failing traces with delta debugging. Out of scope: syscall interception, OS thread scheduling, async timers (see `UNSUPPORTED.md`). Architecture: `ARCHITECTURE.md`.

## Supported platforms and prerequisites

- Rust ≥ 1.85 (verified with 1.85.1 and 1.98.0).
- macOS aarch64: verified. Linux x86_64 and Windows x86_64: CI test suite green (GitHub Actions run 40115734341 on `7b1d772`, 2026-10-24).
- No external services, credentials, hardware or privileges are needed for any test.

## Clean-clone setup (as executed)

```sh
git clone <repo> dtm-clean && cd dtm-clean       # local clone of the commit above, in a scratch dir outside the repo
cargo test
cargo build --examples --bins
./target/debug/examples/flaky_cache --live-runs 30
./target/debug/tm hunt -- ./target/debug/examples/flaky_cache
./target/debug/tm replay tm-out/flaky_cache-run1.tmtrace --times 20 -- ./target/debug/examples/flaky_cache
```

## Commands executed and actual outcomes (clean clone of `7279441`)

| command | exit | result |
|---|---|---|
| `cargo test` | 0 | 76 passed, 0 failed, 1 ignored (fixture regenerator, see below) |
| `cargo build --examples --bins` | 0 | |
| `flaky_cache --live-runs 30` | 0 | 12 failed, 18 passed (real clock; load average 44) |
| `tm hunt -- flaky_cache` | 0 | failure found on run 1, trace saved (8 events) |
| `tm replay <trace> --times 20` | 0 | 20 reproduced, 0 diverged, 0 differed |
| `cargo fmt --all --check` | 0 | |
| `cargo clippy --all-targets --all-features --locked -- -D warnings` | 0 | |
| `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features --locked` | 0 | |
| `cargo build --lib --no-default-features --locked` | 0 | library builds without the CLI's clap/anyhow |
| `cargo deny check` | 0 | advisories, bans, licenses, sources ok |
| `cargo audit` (advisory DB, 1348 advisories) | 0 | 0 vulnerabilities in 66 crate dependencies |
| `cargo +1.85 test --all-features --locked` | 0 | 76 passed, 0 failed, 1 ignored |
| `tm minimize` on `lost_update` (run on `0f4cd12`; code path unchanged since) | 0 | decisions 7 → 1 in 10 program runs |
| `FLAKY_CACHE_VARIANT=extra-draw tm replay --quiet …` (run on `0f4cd12`) | 3 | divergence reported at event #3 (expected) |
| `cargo test --test explore_minimize -- --ignored regenerate` | 0 | regenerated fixture identical in values; only call-site lines differed → refreshed in `8dc646c` |

## Test inventory

| suite | count | what it proves |
|---|---|---|
| unit (`src/`) | 18 | RNG golden values (cross-checked against an independent implementation and the SplitMix64 reference), path confinement, in-memory/real fs, ddmin, failure signatures, trace round trip, site parsing |
| bin unit (`src/bin/tm`) | 4 | scratch dir is 0700 on Unix and removed on drop; trace naming by seed vs run |
| `tests/record_replay.rs` | 9 | property: replay reproduces the observable transcript for 64 random seeds; same seed ⇒ same trace; driving the recorded decisions re-creates the original event stream; replay never consults any source; recorded errors replay as errors; real-clock recording replays without sleeping |
| `tests/divergence.rs` | 13 | each divergence reason, expected/actual/context in the message, caught divergence keeps failing, site-check policies, wrong-type and out-of-range values, checkpoints restore the recorded state in order, reordered checkpoints diverge at the first out-of-order mark |
| `tests/trace_format.rs` | 16 | empty, garbage, wrong format, wrong version, truncation, reordering, removal, checksum, trailing data, duplicate header, count mismatch, atomic save, invalid UTF-8, CRLF, header/footer edits fail replay, no partial file on failed save; properties (256 cases each): random single-byte corruption (file and in-memory readers agree), truncation at any byte, swapping any two events |
| `tests/large_trace.rs` | 4 | over-limit file refused before reading, inclusive configurable limit, endless input (`/dev/zero`) stops at the limit, 50,000-event round trip and owned replay |
| `tests/explore_minimize.rs` | 10 (+1 ignored) | deterministic hunting, failure rate matches the bug (12–21 % band for a 16.7 % bug, 1200 seeds), 50 identical replays, committed fixture replays, race minimized to `[1]`, budget respected, nondeterminism refused with a reason |
| `tests/cli.rs` | 12 | real `tm` binary against the examples: hunt → replay ×10, deterministic hunting, divergence exit 3, fix check over 150 seeds, minimize, validate/inspect/diff, missing-trace explanation, invalid `TM_*` config (6 cases, exit 2), `TM_MAX_TRACE_BYTES` honoured by `tm` and the program, `TM_TEST` filter, stray inherited `TM_TEST` (regression), timeouts |
| `tests/readme_example.rs` | 1 | the README's library example compiles and passes |
| `tests/panic_location.rs` | 1 | divergence panics report the user's call site |
| doctests | 2 | crate-level example and `run_main` example (compile-only) |

Ignored: `regenerate_token_cache_fixture` — a maintenance tool that rewrites a committed fixture; it is not a check and is run explicitly (it passed when run, above).

Security-related tests: path traversal/absolute-path refusal (`RealFs`, `MemFs`), corrupted-trace property test, scratch-dir permissions, inherited-environment isolation, default network refusal (`NoNet`, exercised via the README examples and probe).

## Performance

No benchmarks are published and no performance claims are made. Wall-clock timings in the README (e.g. 20 replays in 0.14 s, 600 simulated runs in 3.9 s) are single observations on a loaded machine, shown only to illustrate that replay does not sleep.

## Bugs found during development and their fixes

| bug | fix | regression test |
|---|---|---|
| Divergence diagnostic printed three times (panic hook, `run_main`, `tm`) | `run_main` suppresses the default hook for divergence panics; `tm replay` prints it only when the child's output is hidden | covered by `tests/cli.rs::changed_program_diverges_with_exit_3` (asserts content), manual check of single print |
| Panic location pointed into the library instead of the user's call | `#[track_caller]` through the engine | `tests/panic_location.rs` (verified to fail without the fix) |
| Real-sources hunt named traces `seed0` although no seed exists | name by run number when the header has no seed | `hunt::tests::real_sources_traces_are_named_by_run_not_by_a_fake_seed` |
| Drive-mode traces claimed `seed 0` | drive traces record no seed | `explore_minimize::minimizer_shrinks_a_race_to_one_decision` (asserts `seed == None`) |
| Exact-message matching prevented minimization when messages contain details | `FailureMatch::Signature` (default) | `minimize::tests::signatures_ignore_details_after_the_colon`, `minimizer_shrinks_a_race_to_one_decision` |
| Inherited `TM_TEST` made children ignore record mode | `tm` strips it unless `--test` is given | `tests/cli.rs::a_stray_tm_test_variable_in_the_users_shell_is_not_inherited` (verified to fail without the fix) |
| Scratch traces world-readable on shared machines | scratch dir created 0700 on Unix | `child::tests::scratch_dir_is_private` |

## Security review (performed by the author; no external audit)

- Secrets scan of all tracked files and history: no matches for token, key or private-key patterns; no absolute local paths.
- Dependency audit: `cargo audit` and `cargo deny` clean (see above).
- Command execution: `tm` runs only the argv after `--` via `std::process::Command` (no shell, so no shell injection).
- Path traversal: `RealFs` refuses `..` and absolute paths (tested); symlinks inside the root are followed (documented).
- Untrusted input: trace parsing is total (errors, no panics), tested with corruption; traces are *inputs* to replayed programs (documented in SECURITY.md).
- Sensitive data: traces may contain what the program read — documented; scratch dir private; `tm-out/` is gitignored.
- Unsafe defaults: none found — simulated sources by default, network disabled by default, no listening sockets. Both the library and the CLI declare `#![forbid(unsafe_code)]`.

## CI status

Workflow `.github/workflows/ci.yml` (lint, test on ubuntu/macos/windows, MSRV 1.85, cargo-deny; read-only permissions; pinned action SHAs). Green on every job — lint, test (ubuntu, macos, windows), msrv, deny — in GitHub Actions run 40115734341 on `7b1d772`, 2026-10-24.

## Known limitations and unresolved risks

See `LIMITATIONS.md` and `UNSUPPORTED.md`. Unresolved: Windows is exercised only by the CI suite; recording holds the trace in memory with no event cap (load is capped at 1 GiB by default); the API has had no outside users.

## Needed for further verification

- Manual use on Linux and Windows beyond the CI suite.
- No hardware, credentials or external services are required.

## Acceptance checklist

| # | item | expected behavior | command / test | result | actual | reference |
|---|---|---|---|---|---|---|
| 1 | Controllable clock and timers | virtual time moves only on sleep; replay never sleeps | `record_replay::virtual_time_advances_only_by_sleeps`, `replay_of_a_real_clock_recording_does_not_sleep` | PASS | replay of a 200 ms real sleep < 150 ms | `src/sources/clock.rs`, `src/ctx/inputs.rs` |
| 2 | Seeded randomness | same seed ⇒ same stream, stable across versions | `rng::tests::seeded_stream_is_stable`, `record_replay::same_seed_same_trace` | PASS | golden values match independent impl | `src/sources/rng.rs` |
| 3 | Injectable fs / env / net | all three recorded and replayed, errors included | `record_replay::replay_reproduces_observable_output`, `recorded_errors_are_replayed_as_errors` | PASS | 64 seeds, identical transcripts | `src/sources/*` |
| 4 | Ordered record/replay | same recorded inputs ⇒ same observable outputs | property test above; `tm replay --times 20` | PASS | 20/20 reproduced | `src/ctx/engine.rs` |
| 5 | Replay detects deliberate divergence | precise diagnostic with expected vs actual, index, site, context | `tests/divergence.rs` (11), CLI exit 3 | PASS | divergence at #3 shown | `src/divergence.rs` |
| 6 | Malformed traces rejected | truncated, corrupted, version-mismatched ⇒ actionable error | `tests/trace_format.rs` (10) | PASS | each case a specific error | `src/trace/` |
| 7 | Failure capture | failing seed + trace saved to a fixture file | `tm hunt`; `explore_minimize::explore_finds_the_same_first_failure_every_time` | PASS | `tm-out/flaky_cache-run1.tmtrace` | `src/harness.rs`, `src/bin/tm/hunt.rs` |
| 8 | Minimization | ddmin shrinks a failing trace that still fails | `minimizer_shrinks_a_race_to_one_decision`, `tm minimize` | PASS | 7 → 1 decisions | `src/minimize.rs` |
| 9 | Flaky test made reproducible | intermittent in normal mode; 100 % with captured trace | `flaky_cache --live-runs 30` then `tm replay --times 20` | PASS | 12/30 live failures; 20/20 replays | `examples/flaky_cache.rs` |
| 10 | Unsupported sources documented | list of uncontrolled nondeterminism | `docs/UNSUPPORTED.md` | PASS | 11 sources listed | — |
| 11 | Clean-clone quick start | README commands work verbatim | clean clone above | PASS | all exits 0 | README |
| 12 | Lint / docs / deps / MSRV | all gates green | table above | PASS | all exit 0 | `.github/workflows/ci.yml` |
| 13 | CI on all OSes | green on Linux, macOS, Windows | GitHub Actions | PASS | GitHub Actions run 40115734341 on `7b1d772`, 2026-10-24 | `.github/workflows/ci.yml` |

## Deep audit addendum (2026-10-22, after `5223f59`)

Scope: the determinism definition, checkpoint ordering, malformed recordings, divergence, memory on large recordings, trace size cap, original-vs-replay comparison. Result: 90 passed, 0 failed, 1 ignored (was 76/0/1).

| finding | severity | fix | regression evidence |
|---|---|---|---|
| `Trace::load` read the whole file with no limit: `tm validate /dev/zero` grew to 1.5 GB in 3 s and never finished | reliability (memory exhaustion from a path argument) | bounded streaming reader, `TraceError::TooLarge`, 1 GiB default, `TM_MAX_TRACE_BYTES` | `tests/large_trace.rs`; old build: 1 GiB + 1 sparse file → read fully (1.07 GB RSS) and reported "malformed"; new build: refused in 0.00 s, 3.7 MB RSS |
| `save` and `load` each held a full copy of the trace text next to the events; `harness::replay` cloned the trace; `run_main` loaded the replayed trace twice | measured memory | streaming save/load, `replay_owned`, outcome read from the replayed trace | 10M events: validate 1,962 → 1,460 MB, replay 2,267 → 1,497 MB (table in `DETERMINISM.md`) |
| `run_main` compared the replay against a second load of `TM_TRACE`, which could differ from the trace replayed if the file was replaced in between | correctness (narrow race) | compare against the trace held by the replaying `Ctx` | structural; covered by the existing CLI replay tests |
| invalid UTF-8 reported as a generic I/O error without a line | error quality | `Malformed { line }` | `invalid_utf8_in_a_file_is_a_malformed_line_not_an_io_error` fails on the old code |
| README claimed "edits" are rejected; the checksum covers event lines only | docs | documented precisely; replay still reports an edited outcome as not reproduced | `header_and_footer_edits_are_not_covered_by_the_checksum_but_fail_replay` |
| save left no fsync before rename | durability | `sync_all` before rename; partial file removed on failure | `save_streams_and_leaves_no_partial_file_on_failure` |

Commands (macOS aarch64, rustc 1.98.0 unless noted), all exit 0: `cargo fmt --all --check`; `cargo clippy --all-targets --all-features --locked -- -D warnings` (1.98.0 and 1.99.1); `cargo test --all-features --locked`; `cargo +1.85 test --all-features --locked`; `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features --locked`; `cargo build --lib --no-default-features --locked`; `cargo deny check`; `cargo audit` (offline DB).

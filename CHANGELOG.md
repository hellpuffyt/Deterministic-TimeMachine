# Changelog

All notable changes are documented here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project uses [Semantic Versioning](https://semver.org/) (0.x: breaking changes may happen in minor versions).

## [Unreleased]

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

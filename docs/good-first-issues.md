# Good first issues

Concrete, self-contained tasks. Each lists where to look and how to know you're done.

1. **`tm hunt --jobs N`** — run several seeds in parallel.
   *Where:* `src/bin/tm/hunt.rs` (each run already has its own trace path pattern; give each job its own scratch file).
   *Done when:* `tm hunt --jobs 4 --sources sim --runs 400 --all -- …` reports the same failure count as `--jobs 1`, and a CLI test covers it.

2. **`tm inspect --json`** — machine-readable output.
   *Where:* `src/bin/tm/view.rs`.
   *Done when:* the output is one JSON document with header, outcome, per-kind counts and events, and a test parses it.

3. **Optional size cap for `Trace::load`.**
   *Where:* `src/trace/codec.rs` (add `load_with_limit`), `src/trace/error.rs` (new error variant with an actionable message).
   *Done when:* a test shows an oversized file is rejected without reading it fully.

4. **`Ctx::gen_range_i64` and `gen_range` for `usize`.**
   *Where:* `src/ctx/inputs.rs`. Build on `next_u64`; do not add new event kinds.
   *Done when:* unit tests cover bounds, and a record → replay test shows the values reproduce.

5. **A `tm replay --diff` mode** that, on a "did not reproduce" result, records the new run and prints `tm diff`-style output against the original trace.
   *Where:* `src/bin/tm/replay.rs`, reusing `Trace::first_difference`.
   *Done when:* a CLI test using `FLAKY_CACHE_VARIANT=fixed` shows the first differing event.

//! The library example from README.md, kept compiling and passing.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::time::Duration;
use timemachine::{Ctx, harness};

fn backoff_fits_deadline(ctx: &Ctx) -> Result<(), String> {
    let start = ctx.now();
    ctx.sleep(Duration::from_millis(90 + ctx.gen_range(0..20)));
    let waited = ctx.now() - start;
    if waited > Duration::from_millis(100) {
        return Err(format!("deadline missed: waited {waited:?}"));
    }
    Ok(())
}

#[test]
fn finds_and_replays_the_failure() {
    // Simulated sources by default: virtual clock (sleeps are instant), seeded RNG.
    let found = harness::explore("backoff", Default::default(), |b| b, backoff_fits_deadline)
        .first_failure
        .expect("about half of all seeds fail");
    let dir = tempfile::tempdir().unwrap();
    found
        .trace
        .save(dir.path().join("backoff.tmtrace"))
        .unwrap();
    // Every replay of the trace fails the same way.
    assert!(harness::replay(&found.trace, |b| b, backoff_fits_deadline).reproduced);
}

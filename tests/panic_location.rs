//! Regression: a divergence panic must point at the user's call site, not
//! into the library. Kept in its own test binary because it installs a
//! process-wide panic hook.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Mutex;

use timemachine::{Builder, Ctx, harness};

static LOCATION: Mutex<Option<String>> = Mutex::new(None);

#[test]
fn divergence_panics_at_the_callers_line() {
    let trace = harness::run(
        &Builder::new("loc").record(),
        |ctx: &Ctx| -> Result<(), String> {
            ctx.next_u64();
            Ok(())
        },
    )
    .trace
    .unwrap();

    std::panic::set_hook(Box::new(|info| {
        *LOCATION.lock().unwrap() = info
            .location()
            .map(|l| format!("{}:{}", l.file(), l.line()));
    }));
    let report = harness::replay(
        &trace,
        |b| b,
        |ctx: &Ctx| -> Result<(), String> {
            let _ = ctx.env_var("NOT_RECORDED"); // diverges here
            Ok(())
        },
    );
    let _ = std::panic::take_hook();
    assert!(report.divergence.is_some());
    let loc = LOCATION.lock().unwrap().clone().unwrap();
    // Before the fix this pointed into src/ctx/engine.rs.
    assert!(
        loc.starts_with("tests/panic_location.rs:"),
        "panic location {loc}"
    );
}

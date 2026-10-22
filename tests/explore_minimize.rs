//! Acceptance: failures are captured from seeds, replay 100% of the time,
//! and minimize.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::cell::Cell;
use std::sync::atomic::{AtomicU64, Ordering};

use timemachine::harness::{self, ExploreConfig};
use timemachine::minimize::{self, FailureMatch, MinimizeConfig, MinimizeError};
use timemachine::sched::{self, Step, Task};
use timemachine::{Builder, Ctx, OutcomeKind, Trace};

#[test]
fn explore_finds_the_same_first_failure_every_time() {
    let a = harness::explore(
        "token",
        ExploreConfig::default(),
        common::token_setup,
        common::token_cache_test,
    );
    let b = harness::explore(
        "token",
        ExploreConfig::default(),
        common::token_setup,
        common::token_cache_test,
    );
    let (fa, fb) = (a.first_failure.unwrap(), b.first_failure.unwrap());
    assert_eq!(fa.seed, fb.seed);
    assert_eq!(fa.trace, fb.trace);
}

#[test]
fn failure_rate_matches_the_bug() {
    // Backoff = 90 + U[0, 12) ms vs a 100 ms TTL: fails iff jitter >= 10.
    let cfg = ExploreConfig {
        runs: 1200,
        first_seed: 0,
        stop_at_first: false,
    };
    let r = harness::explore("token", cfg, common::token_setup, common::token_cache_test);
    let rate = r.failures as f64 / r.runs as f64;
    assert!((0.12..0.21).contains(&rate), "failure rate {rate}");
}

#[test]
fn captured_failure_replays_identically_many_times() {
    let found = harness::explore(
        "token",
        ExploreConfig::default(),
        common::token_setup,
        common::token_cache_test,
    )
    .first_failure
    .unwrap();
    let text = found.trace.to_text();
    for _ in 0..50 {
        let trace = Trace::parse(&text).unwrap();
        let r = harness::replay(&trace, common::token_setup, common::token_cache_test);
        assert!(r.reproduced, "{r:?}");
        assert_eq!(r.outcome, OutcomeKind::Failed);
    }
}

#[test]
fn committed_fixture_still_reproduces() {
    // Regression fixture recorded by `explore` (see docs/ARCHITECTURE.md,
    // "Regenerating fixtures"). If this fails after a code change, the
    // change altered the token-cache test's input sequence.
    let trace = Trace::load(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/token_cache.tmtrace"
    ))
    .unwrap();
    assert_eq!(trace.outcome, OutcomeKind::Failed);
    let r = harness::replay(&trace, common::token_setup, common::token_cache_test);
    assert!(r.reproduced, "{r:?}");
}

/// Lost-update race over the deterministic scheduler.
fn lost_update(ctx: &Ctx) -> Result<(), String> {
    let counter = Cell::new(0u32);
    let tasks: Vec<Task<'_>> = (0..5)
        .map(|w| {
            let counter = &counter;
            let mut read: Option<u32> = None;
            Task::new(format!("w{w}"), move |_: &Ctx| match read {
                None => {
                    read = Some(counter.get());
                    Step::Yield
                }
                Some(v) => {
                    counter.set(v + 1);
                    Step::Done
                }
            })
        })
        .collect();
    let s = sched::run(ctx, tasks, 100);
    if counter.get() != 5 {
        return Err(format!(
            "lost update: {} after 5 increments ({})",
            counter.get(),
            s.steps.join(",")
        ));
    }
    Ok(())
}

#[test]
fn minimizer_shrinks_a_race_to_one_decision() {
    let found = harness::explore("race", ExploreConfig::default(), |b| b, lost_update)
        .first_failure
        .unwrap();
    assert!(found.trace.decisions().len() >= 5);
    let m =
        minimize::minimize(&found.trace, MinimizeConfig::default(), |b| b, lost_update).unwrap();
    assert_eq!(
        m.decisions,
        vec![1],
        "one out-of-order read is the minimal race"
    );
    assert!(m.runs < 100, "used {} runs", m.runs);
    // The minimized trace is itself a reproducible failure.
    assert!(harness::replay(&m.trace, |b| b, lost_update).reproduced);
    assert!(m.trace.message.unwrap().starts_with("lost update:"));
    // Regression: drive-mode traces used to claim "seed 0".
    assert_eq!(m.trace.header.seed, None);
    assert_eq!(m.trace.header.sources, "drive");
}

#[test]
fn sequential_schedule_passes() {
    // Exhausted decisions mean "always the first runnable task": sequential.
    let out = harness::run(&Builder::new("race").drive(vec![]), lost_update);
    assert_eq!(out.kind, OutcomeKind::Passed);
}

#[test]
fn minimizing_a_passing_trace_is_refused() {
    let ctx = Builder::new("race").drive(vec![]);
    let trace = harness::run(&ctx, lost_update).trace.unwrap();
    let err =
        minimize::minimize(&trace, MinimizeConfig::default(), |b| b, lost_update).unwrap_err();
    assert!(matches!(
        err,
        MinimizeError::NotAFailure(OutcomeKind::Passed)
    ));
}

#[test]
fn nondeterministic_failures_are_refused_with_a_reason() {
    // This test only fails on its first execution: hidden global state that
    // timemachine cannot see.
    static CALLS: AtomicU64 = AtomicU64::new(0);
    let flaky = |ctx: &Ctx| -> Result<(), String> {
        let _ = ctx.next_u64();
        if CALLS.fetch_add(1, Ordering::SeqCst) == 0 {
            Err("first call fails".into())
        } else {
            Ok(())
        }
    };
    let trace = harness::run(&Builder::new("hidden").record(), flaky)
        .trace
        .unwrap();
    let err = minimize::minimize(&trace, MinimizeConfig::default(), |b| b, flaky).unwrap_err();
    assert!(matches!(err, MinimizeError::DoesNotReproduce { .. }));
    assert!(err.to_string().contains("not deterministic"));
}

#[test]
fn run_budget_is_respected() {
    let found = harness::explore("race", ExploreConfig::default(), |b| b, lost_update)
        .first_failure
        .unwrap();
    let cfg = MinimizeConfig {
        max_runs: 3,
        matching: FailureMatch::Signature,
    };
    let m = minimize::minimize(&found.trace, cfg, |b| b, lost_update).unwrap();
    // 3 budgeted runs + the initial check + the confirmation run.
    assert!(m.runs <= 5, "used {} runs", m.runs);
}

#[test]
fn panics_are_failures_with_their_message() {
    let out = harness::run(
        &Builder::new("p").record(),
        |ctx: &Ctx| -> Result<(), String> {
            let v = ctx.gen_range(0..1);
            assert_eq!(v, 1, "deliberate");
            Ok(())
        },
    );
    assert_eq!(out.kind, OutcomeKind::Panicked);
    assert!(out.message.unwrap().contains("deliberate"));
    assert_eq!(out.trace.unwrap().outcome, OutcomeKind::Panicked);
}

/// Writes `tests/fixtures/token_cache.tmtrace`. Not a check: run it on
/// purpose (`cargo test --test explore_minimize -- --ignored regenerate`)
/// after an intentional change to the token-cache test's input sequence.
#[test]
#[ignore = "maintenance tool: regenerates a committed fixture, run explicitly"]
fn regenerate_token_cache_fixture() {
    let found = harness::explore(
        "token",
        ExploreConfig::default(),
        common::token_setup,
        common::token_cache_test,
    )
    .first_failure
    .unwrap();
    found
        .trace
        .save(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/token_cache.tmtrace"
        ))
        .unwrap();
}

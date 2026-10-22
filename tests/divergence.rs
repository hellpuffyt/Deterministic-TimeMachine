//! Acceptance: replay detects deliberate divergence and explains it.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::time::Duration;

use timemachine::{Builder, Ctx, DivergenceReason, OutcomeKind, SiteCheck, Trace, harness};

fn record(test: impl FnOnce(&Ctx) -> Result<(), String>) -> Trace {
    let ctx = Builder::new("t").seed(1).record();
    harness::run(&ctx, test).trace.unwrap()
}

fn base(ctx: &Ctx) -> Result<(), String> {
    ctx.mark("begin");
    let _ = ctx.env_var("RETRIES");
    ctx.sleep(Duration::from_millis(5));
    let _ = ctx.next_u64();
    ctx.mark("end");
    Ok(())
}

fn replay(trace: &Trace, test: impl FnOnce(&Ctx) -> Result<(), String>) -> harness::ReplayReport {
    harness::replay(trace, |b| b, test)
}

#[test]
fn identical_program_reproduces() {
    let t = record(base);
    let r = replay(&t, base);
    assert!(r.reproduced, "{:?}", r.divergence);
}

#[test]
fn different_key_is_reported_with_expected_and_actual() {
    let t = record(base);
    let r = replay(&t, |ctx| {
        ctx.mark("begin");
        let _ = ctx.env_var("RETRY_COUNT"); // renamed variable
        Ok(())
    });
    let d = r.divergence.expect("divergence");
    assert_eq!(r.outcome, OutcomeKind::Diverged);
    assert_eq!(d.reason, DivergenceReason::DifferentRequest);
    assert_eq!(d.index, 1);
    assert_eq!(d.expected.as_ref().unwrap().key.as_deref(), Some("RETRIES"));
    assert_eq!(
        d.actual.as_ref().unwrap().key.as_deref(),
        Some("RETRY_COUNT")
    );
    assert_eq!(d.context.len(), 1, "the mark before it is shown as context");
    let text = d.to_string();
    for needle in [
        "event #1 of 5",
        "expected: #1 env.var \"RETRIES\"",
        "actual:   #1 env.var \"RETRY_COUNT\"",
        "#0 mark \"begin\"",
        "hint:",
    ] {
        assert!(text.contains(needle), "missing {needle:?} in:\n{text}");
    }
}

#[test]
fn different_kind_is_reported() {
    let t = record(base);
    let r = replay(&t, |ctx| {
        ctx.mark("begin");
        let _ = ctx.next_u64(); // skipped the env lookup
        Ok(())
    });
    let d = r.divergence.unwrap();
    assert_eq!(d.reason, DivergenceReason::DifferentRequest);
    assert_eq!(d.expected.unwrap().kind, timemachine::Kind::EnvVar);
    assert_eq!(d.actual.unwrap().kind, timemachine::Kind::RngU64);
}

#[test]
fn changed_sleep_duration_is_a_divergence() {
    let t = record(base);
    let r = replay(&t, |ctx| {
        ctx.mark("begin");
        let _ = ctx.env_var("RETRIES");
        ctx.sleep(Duration::from_millis(6));
        Ok(())
    });
    assert_eq!(r.divergence.unwrap().index, 2);
}

#[test]
fn asking_for_more_inputs_than_recorded() {
    let t = record(base);
    let r = replay(&t, |ctx| {
        base(ctx)?;
        let _ = ctx.next_u64();
        Ok(())
    });
    let d = r.divergence.unwrap();
    assert_eq!(d.reason, DivergenceReason::PastEnd);
    assert_eq!(d.index, 5);
    assert!(d.to_string().contains("expected: <end of trace>"));
}

#[test]
fn finishing_early_is_a_divergence() {
    let t = record(base);
    let r = replay(&t, |ctx| {
        ctx.mark("begin");
        Ok(())
    });
    let d = r.divergence.unwrap();
    assert_eq!(d.reason, DivergenceReason::EndedEarly);
    assert_eq!(d.index, 1);
}

#[test]
fn a_caught_divergence_keeps_failing() {
    let t = record(base);
    let r = replay(&t, |ctx| {
        ctx.mark("begin");
        // The program swallows the first divergence panic…
        let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| ctx.next_u64()));
        assert!(caught.is_err());
        // …but every later input request fails too.
        let _ = ctx.env_var("RETRIES");
        Ok(())
    });
    assert_eq!(r.outcome, OutcomeKind::Diverged);
}

fn site_a(ctx: &Ctx) -> u64 {
    ctx.next_u64()
}
fn site_b(ctx: &Ctx) -> u64 {
    ctx.next_u64()
}

#[test]
fn site_checking_policies() {
    let t = record(|ctx| {
        site_a(ctx);
        Ok(())
    });
    let moved = |ctx: &Ctx| -> Result<(), String> {
        site_b(ctx);
        Ok(())
    };
    // Same file, different line: allowed by default, rejected by Exact.
    assert!(harness::replay(&t, |b| b, moved).reproduced);
    let strict = harness::replay(&t, |b| b.site_check(SiteCheck::Exact), moved);
    assert_eq!(
        strict.divergence.unwrap().reason,
        DivergenceReason::DifferentSite
    );
    // A different file is rejected by File but accepted by Off.
    let mut other_file = t.clone();
    other_file.events[0].site = "src/elsewhere.rs:1:1".into();
    let file = harness::replay(&other_file, |b| b, moved);
    assert_eq!(
        file.divergence.unwrap().reason,
        DivergenceReason::DifferentSite
    );
    assert!(harness::replay(&other_file, |b| b.site_check(SiteCheck::Off), moved).reproduced);
}

#[test]
fn a_value_of_the_wrong_type_is_reported_not_misread() {
    let t = record(base);
    let mut edited = t.clone();
    edited.events[3].value = timemachine::trace::Value::Bool(true); // rng.u64 now holds a bool
    let r = replay(&edited, base);
    let d = r.divergence.unwrap();
    assert_eq!(d.reason, DivergenceReason::BadValue);
    assert!(d.detail.unwrap().contains("expected a u64 value"));
}

#[test]
fn an_out_of_range_choice_is_reported() {
    let t = record(|ctx| {
        ctx.choose(3);
        Ok(())
    });
    let mut edited = t.clone();
    edited.events[0].value = timemachine::trace::Value::U64(7);
    let r = harness::replay(
        &edited,
        |b| b,
        |ctx: &Ctx| -> Result<(), String> {
            ctx.choose(3);
            Ok(())
        },
    );
    assert_eq!(r.divergence.unwrap().reason, DivergenceReason::BadValue);
}

#[test]
fn a_different_failure_is_not_reported_as_reproduced() {
    let t = record(|ctx| {
        let v = ctx.gen_range(0..10);
        Err(format!("bad value {v}"))
    });
    let r = harness::replay(
        &t,
        |b| b,
        |ctx: &Ctx| -> Result<(), String> {
            let _ = ctx.gen_range(0..10);
            Err("some other failure".into())
        },
    );
    assert!(r.divergence.is_none());
    assert!(!r.reproduced);
}

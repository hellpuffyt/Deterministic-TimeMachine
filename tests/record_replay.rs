//! Acceptance: the same recorded inputs produce the same observable outputs.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::time::Duration;

use proptest::prelude::*;
use timemachine::sources::{Clock, Env, Fs, Net, NetError, Request, Response, RngSource};
use timemachine::{Builder, Ctx, Mode, OutcomeKind, Trace, harness};

fn record_probe(seed: u64) -> (String, Trace) {
    let ctx = common::rich_setup(Builder::new("probe").seed(seed)).record();
    let out = common::probe(&ctx).unwrap();
    let trace = ctx.finish(OutcomeKind::Passed, None).unwrap().unwrap();
    (out, trace)
}

fn replay_probe(trace: &Trace, setup: impl FnOnce(Builder) -> Builder) -> String {
    let ctx = setup(Builder::new("probe")).replay(trace.clone());
    let out = common::probe(&ctx).unwrap();
    assert!(ctx.finish(OutcomeKind::Passed, None).unwrap().is_none());
    out
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// For any seed: replaying the recording reproduces the exact transcript,
    /// and the trace survives a write/read round trip unchanged.
    #[test]
    fn replay_reproduces_observable_output(seed in any::<u64>()) {
        let (recorded, trace) = record_probe(seed);
        let reparsed = Trace::parse(&trace.to_text()).unwrap();
        prop_assert_eq!(&reparsed, &trace);
        prop_assert_eq!(replay_probe(&reparsed, |b| b), recorded);
    }

    /// Simulated sources are a pure function of the seed.
    #[test]
    fn same_seed_same_trace(seed in any::<u64>()) {
        let (a, ta) = record_probe(seed);
        let (b, tb) = record_probe(seed);
        prop_assert_eq!(a, b);
        prop_assert_eq!(ta.first_difference(&tb), None);
    }

    /// Original vs re-execution: driving a fresh run with the recorded
    /// decisions (and the same simulated sources) re-creates the original
    /// event stream value for value, and the same observable output. This is
    /// the property the minimizer relies on.
    #[test]
    fn driving_the_recorded_decisions_recreates_the_original_run(seed in any::<u64>()) {
        let (recorded, trace) = record_probe(seed);
        let ctx = common::rich_setup(Builder::new("probe").seed(seed)).drive(trace.decisions());
        let out = common::probe(&ctx).unwrap();
        let driven = ctx.finish(OutcomeKind::Passed, None).unwrap().unwrap();
        prop_assert_eq!(out, recorded);
        prop_assert_eq!(trace.first_difference(&driven), None);
        prop_assert_eq!(driven.events.len(), trace.events.len());
    }
}

#[test]
fn different_seeds_take_different_paths() {
    let distinct: std::collections::HashSet<String> = (0..20).map(|s| record_probe(s).0).collect();
    assert!(
        distinct.len() > 15,
        "only {} distinct transcripts",
        distinct.len()
    );
}

/// Sources that fail the test if they are ever consulted.
struct Forbidden;
impl Clock for Forbidden {
    fn now_nanos(&mut self) -> u64 {
        panic!("replay consulted the clock")
    }
    fn sleep(&mut self, _: Duration) {
        panic!("replay slept")
    }
}
impl RngSource for Forbidden {
    fn next_u64(&mut self) -> u64 {
        panic!("replay consulted the rng")
    }
}
impl Env for Forbidden {
    fn var(&mut self, _: &str) -> Option<String> {
        panic!("replay consulted the environment")
    }
}
impl Fs for Forbidden {
    fn read(&mut self, _: &std::path::Path) -> std::io::Result<Vec<u8>> {
        panic!("replay read a file")
    }
    fn write(&mut self, _: &std::path::Path, _: &[u8]) -> std::io::Result<()> {
        panic!("replay wrote a file")
    }
    fn exists(&mut self, _: &std::path::Path) -> bool {
        panic!("replay checked a file")
    }
    fn remove(&mut self, _: &std::path::Path) -> std::io::Result<()> {
        panic!("replay removed a file")
    }
    fn list(&mut self, _: &std::path::Path) -> std::io::Result<Vec<String>> {
        panic!("replay listed a directory")
    }
}
impl Net for Forbidden {
    fn request(&mut self, _: &Request) -> Result<Response, NetError> {
        panic!("replay used the network")
    }
}

#[test]
fn replay_never_consults_sources() {
    let (recorded, trace) = record_probe(42);
    let replayed = replay_probe(&trace, |b| {
        b.clock(Forbidden)
            .rng(Forbidden)
            .env(Forbidden)
            .fs(Forbidden)
            .net(Forbidden)
    });
    assert_eq!(replayed, recorded);
}

#[test]
fn recorded_errors_are_replayed_as_errors() {
    let (recorded, trace) = record_probe(7);
    assert!(recorded.contains("absent=Err(NotFound)"), "{recorded}");
    assert!(
        recorded.contains("escape=Err(PermissionDenied)"),
        "{recorded}"
    );
    assert!(recorded.contains("net=Err(NetError"), "{recorded}");
    assert_eq!(replay_probe(&trace, |b| b), recorded);
}

#[test]
fn replay_of_a_real_clock_recording_does_not_sleep() {
    let test = |ctx: &Ctx| -> Result<(), String> {
        ctx.sleep(Duration::from_millis(200));
        Ok(())
    };
    let start = std::time::Instant::now();
    let out = harness::run(&Builder::new("sleepy").real_sources().record(), test);
    assert_eq!(out.kind, OutcomeKind::Passed);
    assert!(
        start.elapsed() >= Duration::from_millis(200),
        "recording must really sleep"
    );
    let trace = out.trace.unwrap();
    assert_eq!(trace.header.seed, None);
    assert_eq!(trace.header.sources, "real");

    let start = std::time::Instant::now();
    let report = harness::replay(&trace, |b| b, test);
    assert!(report.reproduced);
    assert!(
        start.elapsed() < Duration::from_millis(150),
        "replay took {:?}",
        start.elapsed()
    );
}

#[test]
fn live_mode_records_nothing() {
    let ctx = common::rich_setup(Builder::new("live")).live();
    assert_eq!(ctx.mode(), Mode::Live);
    common::probe(&ctx).unwrap();
    assert_eq!(ctx.position(), 0);
    assert!(ctx.finish(OutcomeKind::Passed, None).unwrap().is_none());
}

#[test]
fn virtual_time_advances_only_by_sleeps() {
    let ctx = Builder::new("time").record();
    let a = ctx.now();
    ctx.sleep(Duration::from_secs(3600));
    assert_eq!(ctx.now() - a, Duration::from_secs(3600));
    assert_eq!(
        a,
        Duration::from_nanos(timemachine::sources::clock::DEFAULT_EPOCH_NANOS)
    );
}

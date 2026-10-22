//! In-process helpers: run a test under a context, hunt for a failing seed,
//! and replay a captured trace.

use std::fmt::Display;
use std::panic::{AssertUnwindSafe, catch_unwind};

use crate::ctx::{Builder, Ctx};
use crate::divergence::Divergence;
use crate::trace::{OutcomeKind, Trace};

/// The result of running a test once.
#[derive(Debug, Clone)]
pub struct RunOutcome {
    /// How the run ended.
    pub kind: OutcomeKind,
    /// Failure or divergence message.
    pub message: Option<String>,
    /// The recorded trace (record and drive modes).
    pub trace: Option<Trace>,
    /// The divergence, if replay detected one.
    pub divergence: Option<Divergence>,
}

fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_owned()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "panic with a non-string payload".to_owned()
    }
}

/// Runs `test` once under `ctx`, catching panics, and finishes the context.
pub fn run<F, E>(ctx: &Ctx, test: F) -> RunOutcome
where
    F: FnOnce(&Ctx) -> Result<(), E>,
    E: Display,
{
    let result = catch_unwind(AssertUnwindSafe(|| test(ctx)));
    let (kind, message) = match result {
        Ok(Ok(())) => (OutcomeKind::Passed, None),
        Ok(Err(e)) => (OutcomeKind::Failed, Some(e.to_string())),
        Err(payload) => (OutcomeKind::Panicked, Some(panic_message(payload.as_ref()))),
    };
    match ctx.finish(kind, message.clone()) {
        Ok(trace) => RunOutcome {
            kind,
            message,
            trace,
            divergence: None,
        },
        Err(d) => RunOutcome {
            kind: OutcomeKind::Diverged,
            message: Some(d.to_string()),
            trace: None,
            divergence: Some(*d),
        },
    }
}

/// A failure found by [`explore`].
#[derive(Debug, Clone)]
pub struct Found {
    /// Seed that produced the failure.
    pub seed: u64,
    /// The recorded trace of the failing run.
    pub trace: Trace,
}

/// Summary of an [`explore`] call.
#[derive(Debug, Clone)]
pub struct ExploreReport {
    /// Runs executed.
    pub runs: u64,
    /// Runs that failed or panicked.
    pub failures: u64,
    /// The first failure, if any.
    pub first_failure: Option<Found>,
}

/// Options for [`explore`].
#[derive(Debug, Clone, Copy)]
pub struct ExploreConfig {
    /// Number of seeds to try.
    pub runs: u64,
    /// First seed; seeds are `first_seed..first_seed + runs`.
    pub first_seed: u64,
    /// Stop at the first failure.
    pub stop_at_first: bool,
}

impl Default for ExploreConfig {
    fn default() -> Self {
        ExploreConfig {
            runs: 100,
            first_seed: 0,
            stop_at_first: true,
        }
    }
}

/// Runs `test` with seeds `first_seed..first_seed + runs` in record mode,
/// using the sources configured by `setup` (simulated by default), and
/// returns the first failing trace.
pub fn explore<S, F, E>(test_name: &str, config: ExploreConfig, setup: S, test: F) -> ExploreReport
where
    S: Fn(Builder) -> Builder,
    F: Fn(&Ctx) -> Result<(), E>,
    E: Display,
{
    let mut report = ExploreReport {
        runs: 0,
        failures: 0,
        first_failure: None,
    };
    for seed in config.first_seed..config.first_seed.saturating_add(config.runs) {
        let ctx = setup(Builder::new(test_name).seed(seed)).record();
        let outcome = run(&ctx, &test);
        report.runs += 1;
        if outcome.kind.is_failure() {
            report.failures += 1;
            if report.first_failure.is_none() {
                if let Some(trace) = outcome.trace {
                    report.first_failure = Some(Found { seed, trace });
                }
            }
            if config.stop_at_first {
                break;
            }
        }
    }
    report
}

/// The result of replaying a trace.
#[derive(Debug, Clone)]
pub struct ReplayReport {
    /// How the replayed run ended.
    pub outcome: OutcomeKind,
    /// Its message.
    pub message: Option<String>,
    /// The divergence, if one was detected.
    pub divergence: Option<Divergence>,
    /// Whether the replay ended exactly as the recording did (same outcome
    /// and message, no divergence).
    pub reproduced: bool,
}

/// Replays `trace` against `test`.
pub fn replay<S, F, E>(trace: &Trace, setup: S, test: F) -> ReplayReport
where
    S: Fn(Builder) -> Builder,
    F: FnOnce(&Ctx) -> Result<(), E>,
    E: Display,
{
    replay_owned(trace.clone(), setup, test)
}

/// Replays `trace` against `test`, taking ownership of the trace so a large
/// recording is not copied (peak memory is one trace, not two).
pub fn replay_owned<S, F, E>(trace: Trace, setup: S, test: F) -> ReplayReport
where
    S: Fn(Builder) -> Builder,
    F: FnOnce(&Ctx) -> Result<(), E>,
    E: Display,
{
    let recorded_outcome = trace.outcome;
    let recorded_message = trace.message.clone();
    let ctx = setup(Builder::new(trace.header.test.clone())).replay(trace);
    let out = run(&ctx, test);
    let reproduced =
        out.divergence.is_none() && out.kind == recorded_outcome && out.message == recorded_message;
    ReplayReport {
        outcome: out.kind,
        message: out.message,
        divergence: out.divergence,
        reproduced,
    }
}

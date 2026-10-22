//! Failure minimization by delta debugging over decisions.
//!
//! A trace's *decisions* are the values the program drew (random numbers and
//! [`crate::Ctx::choose`] results). Re-running the program in drive mode with a
//! shorter or smaller decision list produces a different — but valid —
//! execution: once the list runs out every draw is `0`. The minimizer looks
//! for the smallest decision list that still fails the same way, using
//! Zeller's ddmin followed by a value-shrinking pass.
//!
//! This requires the program's *other* inputs (clock, files, environment,
//! network) to be deterministic during minimization, which is the case with
//! the default simulated sources.

use std::fmt::Display;

use crate::ctx::{Builder, Ctx};
use crate::harness::run;
use crate::trace::{OutcomeKind, Trace};

/// Classic ddmin: returns a 1-minimal sub-sequence of `input` for which
/// `fails` still returns `true` (assuming `fails(input)` is true).
///
/// `fails` is called with candidate sub-sequences; it may enforce its own
/// run budget by returning `false` once exhausted.
pub fn ddmin<T: Clone>(input: Vec<T>, mut fails: impl FnMut(&[T]) -> bool) -> Vec<T> {
    let mut input = input;
    let empty: Vec<T> = Vec::new();
    if input.is_empty() || fails(&empty) {
        return empty;
    }
    let mut n = 2usize;
    while input.len() >= 2 {
        let len = input.len();
        let chunk = len.div_ceil(n);
        let subsets: Vec<std::ops::Range<usize>> = (0..len)
            .step_by(chunk)
            .map(|start| start..(start + chunk).min(len))
            .collect();
        let mut progressed = false;
        for r in &subsets {
            let candidate = input[r.clone()].to_vec();
            if fails(&candidate) {
                input = candidate;
                n = 2;
                progressed = true;
                break;
            }
        }
        if !progressed && subsets.len() > 2 {
            for r in &subsets {
                let candidate: Vec<T> = input[..r.start]
                    .iter()
                    .chain(&input[r.end..])
                    .cloned()
                    .collect();
                if fails(&candidate) {
                    input = candidate;
                    n = (n - 1).max(2);
                    progressed = true;
                    break;
                }
            }
        }
        if !progressed {
            if n >= len {
                break;
            }
            n = (n * 2).min(len);
        }
    }
    input
}

/// Shrinks a failing decision list: ddmin, then a value pass that tries `0`
/// and repeated halving for each decision, then removal of trailing zeros
/// (an exhausted list yields zeros anyway). `fails` must return `true` for
/// `original`.
pub fn shrink_decisions(original: Vec<u64>, mut fails: impl FnMut(&[u64]) -> bool) -> Vec<u64> {
    let mut best = ddmin(original, &mut fails);
    for i in 0..best.len() {
        if best[i] == 0 {
            continue;
        }
        let saved = best[i];
        best[i] = 0;
        if fails(&best) {
            continue;
        }
        best[i] = saved;
        loop {
            let half = best[i] / 2;
            if half == best[i] {
                break;
            }
            let prev = best[i];
            best[i] = half;
            if !fails(&best) {
                best[i] = prev;
                break;
            }
        }
    }
    while best.last() == Some(&0) {
        best.pop();
    }
    best
}

/// Which failures count as "the same failure" while minimizing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FailureMatch {
    /// Messages must agree up to their first `:` (the failure's
    /// [`signature`]). Put variable details after a colon:
    /// `"lost update: counter is 3"`. This is the default.
    #[default]
    Signature,
    /// Messages must be identical.
    Exact,
    /// Any failure counts (the minimizer may slip to a different bug).
    Any,
}

impl FailureMatch {
    /// Whether `candidate` counts as the same failure as `target`.
    pub fn matches(self, target: Option<&str>, candidate: Option<&str>) -> bool {
        match self {
            FailureMatch::Any => true,
            FailureMatch::Exact => target == candidate,
            FailureMatch::Signature => target.map(signature) == candidate.map(signature),
        }
    }
}

/// The part of a failure message before the first `:`, trimmed.
pub fn signature(message: &str) -> &str {
    message.split(':').next().unwrap_or(message).trim()
}

/// Options for [`minimize`].
#[derive(Debug, Clone, Copy)]
pub struct MinimizeConfig {
    /// Maximum number of test executions.
    pub max_runs: u64,
    /// Which failures count as the original one.
    pub matching: FailureMatch,
}

impl Default for MinimizeConfig {
    fn default() -> Self {
        MinimizeConfig {
            max_runs: 5_000,
            matching: FailureMatch::Signature,
        }
    }
}

/// A minimized failure.
#[derive(Debug, Clone)]
pub struct Minimized {
    /// Decisions of the original trace.
    pub original: Vec<u64>,
    /// Minimized decisions (trailing zeros removed).
    pub decisions: Vec<u64>,
    /// Test executions used.
    pub runs: u64,
    /// The recorded trace of the minimized failing run; replayable.
    pub trace: Trace,
}

/// Why minimization could not start or finish.
#[derive(Debug, thiserror::Error)]
pub enum MinimizeError {
    /// The trace records a passing run.
    #[error("the trace records a {0} run; only failing traces can be minimized")]
    NotAFailure(OutcomeKind),
    /// Driving the original decisions does not reproduce the failure.
    #[error(
        "re-running the original decisions did not reproduce the failure (got {outcome}: {message}); the test's non-random inputs are probably not deterministic — use simulated sources"
    )]
    DoesNotReproduce {
        /// Outcome observed.
        outcome: OutcomeKind,
        /// Message observed.
        message: String,
    },
}

/// Shrinks the decisions of a failing `trace` while `test` keeps failing.
pub fn minimize<S, F, E>(
    trace: &Trace,
    config: MinimizeConfig,
    setup: S,
    test: F,
) -> Result<Minimized, MinimizeError>
where
    S: Fn(Builder) -> Builder,
    F: Fn(&Ctx) -> Result<(), E>,
    E: Display,
{
    if !trace.outcome.is_failure() {
        return Err(MinimizeError::NotAFailure(trace.outcome));
    }
    let name = trace.header.test.clone();
    let target = trace.message.clone();
    let mut runs = 0u64;
    let mut attempt = |decisions: &[u64]| {
        runs += 1;
        let ctx = setup(Builder::new(name.clone())).drive(decisions.to_vec());
        run(&ctx, &test)
    };
    let original = trace.decisions();
    let first = attempt(&original);
    if !(first.kind.is_failure()
        && config
            .matching
            .matches(target.as_deref(), first.message.as_deref()))
    {
        return Err(MinimizeError::DoesNotReproduce {
            outcome: first.kind,
            message: first.message.unwrap_or_default(),
        });
    }

    let mut budget_left = config.max_runs.saturating_sub(1);
    let mut fails = |d: &[u64]| -> bool {
        if budget_left == 0 {
            return false;
        }
        budget_left -= 1;
        let out = attempt(d);
        out.kind.is_failure()
            && config
                .matching
                .matches(target.as_deref(), out.message.as_deref())
    };

    let best = shrink_decisions(original.clone(), &mut fails);
    let last = attempt(&best);
    let trace = match last.trace {
        Some(t) if last.kind.is_failure() => t,
        _ => {
            return Err(MinimizeError::DoesNotReproduce {
                outcome: last.kind,
                message: last.message.unwrap_or_default(),
            });
        }
    };
    Ok(Minimized {
        original,
        decisions: best,
        runs,
        trace,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ddmin_finds_the_two_relevant_elements() {
        let input: Vec<u32> = (0..64).collect();
        let mut calls = 0;
        let out = ddmin(input, |c| {
            calls += 1;
            c.contains(&17) && c.contains(&42)
        });
        assert_eq!(out, vec![17, 42]);
        assert!(calls < 200, "ddmin used {calls} calls");
    }

    #[test]
    fn signatures_ignore_details_after_the_colon() {
        assert_eq!(signature("lost update: counter is 3"), "lost update");
        assert_eq!(signature("no colon here"), "no colon here");
        let m = FailureMatch::Signature;
        assert!(m.matches(Some("x: 1"), Some("x: 2")));
        assert!(!m.matches(Some("x: 1"), Some("y: 1")));
        assert!(!FailureMatch::Exact.matches(Some("x: 1"), Some("x: 2")));
        assert!(FailureMatch::Any.matches(Some("x"), None));
    }

    #[test]
    fn ddmin_handles_trivially_failing_input() {
        assert!(ddmin(vec![1, 2, 3], |_| true).is_empty());
        assert_eq!(ddmin(vec![5], |c| c == [5]), vec![5]);
    }
}

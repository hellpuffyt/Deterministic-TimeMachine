//! [`Ctx`]: the handle through which a program reads every nondeterministic
//! input, and the record / replay / drive engine behind it.
//!
//! * `builder` — configuring sources and choosing a mode ([`Builder`]),
//! * `engine` — the record/replay/drive state machine and divergence detection,
//! * `inputs` — the public input methods (`now`, `next_u64`, `read`, …).

mod builder;
mod engine;
mod inputs;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

pub use builder::Builder;

use crate::divergence::{Divergence, DivergenceReason};
use crate::trace::{Header, OutcomeKind, Trace};
use engine::{Engine, State};

/// How a [`Ctx`] obtains inputs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Ask the sources; record nothing.
    Live,
    /// Ask the sources and record every answer.
    Record,
    /// Answer from a recorded trace; never ask the sources.
    Replay,
    /// Take decisions (random draws, choices) from a given list, ask the
    /// sources for everything else, and record. Used by the minimizer.
    Drive,
}

/// How strictly replay compares call sites.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SiteCheck {
    /// File, line and column must match.
    Exact,
    /// The file must match; line and column may move (default — editing
    /// unrelated lines does not invalidate a trace).
    #[default]
    File,
    /// Call sites are not compared.
    Off,
}

/// Where an environment-driven run writes its artifacts.
#[derive(Debug, Clone, Default)]
pub(crate) struct Persist {
    pub(crate) trace_out: Option<PathBuf>,
    pub(crate) report_out: Option<PathBuf>,
}

struct Inner {
    state: Mutex<State>,
    test: String,
    seed: Option<u64>,
    sources_label: String,
    site_check: SiteCheck,
    mode: Mode,
    persist: Persist,
}

/// The handle a program uses for time, randomness, files, environment and
/// network. Cheap to clone; clones share one recording.
///
/// Calls must happen in a deterministic order for replay to work: use a
/// `Ctx` from one thread, or serialize access (see [`crate::sched`]).
#[derive(Clone)]
pub struct Ctx {
    inner: Arc<Inner>,
}

impl Ctx {
    /// The mode this context runs in.
    pub fn mode(&self) -> Mode {
        self.inner.mode
    }

    /// The test name.
    pub fn test_name(&self) -> &str {
        &self.inner.test
    }

    /// Seed of the simulated random source, if one is used.
    pub fn seed(&self) -> Option<u64> {
        self.inner.seed
    }

    /// The divergence detected so far, if any.
    pub fn divergence(&self) -> Option<Divergence> {
        self.lock().divergence.clone()
    }

    /// Number of events recorded (record/drive) or consumed (replay).
    pub fn position(&self) -> usize {
        match &self.lock().engine {
            Engine::Live => 0,
            Engine::Record(e) | Engine::Drive { events: e, .. } => e.len(),
            Engine::Replay { cursor, .. } => *cursor,
        }
    }

    /// Ends the run.
    ///
    /// * record / drive: returns the trace (with the given outcome);
    /// * replay: checks that every recorded input was used, returning the
    ///   divergence otherwise (or the one detected earlier);
    /// * live: returns `Ok(None)`.
    ///
    /// Environment-driven contexts also write the trace to `TM_TRACE`.
    pub fn finish(
        &self,
        outcome: OutcomeKind,
        message: Option<String>,
    ) -> Result<Option<Trace>, Box<Divergence>> {
        let mut guard = self.lock();
        if guard.finished {
            return Ok(None);
        }
        guard.finished = true;
        if let Some(d) = guard.divergence.clone() {
            return Err(Box::new(d));
        }
        let header = Header::new(
            self.inner.test.clone(),
            self.inner.seed,
            self.inner.sources_label.clone(),
        );
        let trace = match &mut guard.engine {
            Engine::Live => None,
            Engine::Record(events) | Engine::Drive { events, .. } => Some(Trace {
                header,
                events: std::mem::take(events),
                outcome,
                message,
            }),
            Engine::Replay { trace, cursor } => {
                if *cursor < trace.events.len() && !outcome.is_failure() {
                    let d = self.make_divergence(
                        trace,
                        *cursor,
                        DivergenceReason::EndedEarly,
                        None,
                        None,
                    );
                    guard.divergence = Some(d.clone());
                    return Err(Box::new(d));
                }
                None
            }
        };
        drop(guard);
        if let (Some(t), Some(path)) = (&trace, &self.inner.persist.trace_out) {
            let _ = t.save(path);
        }
        Ok(trace)
    }
}

impl Drop for Inner {
    fn drop(&mut self) {
        // Environment-driven runs that never called `finish` (e.g. a plain
        // `#[test]` that panicked) still leave their trace behind.
        let Some(path) = &self.persist.trace_out else {
            return;
        };
        let state = self.state.get_mut().unwrap_or_else(|p| p.into_inner());
        if state.finished {
            return;
        }
        let events = match &mut state.engine {
            Engine::Record(e) | Engine::Drive { events: e, .. } => std::mem::take(e),
            _ => return,
        };
        let outcome = if std::thread::panicking() {
            OutcomeKind::Panicked
        } else {
            OutcomeKind::Unknown
        };
        let trace = Trace {
            header: Header::new(self.test.clone(), self.seed, self.sources_label.clone()),
            events,
            outcome,
            message: None,
        };
        let _ = trace.save(path);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn finish_is_idempotent() {
        let ctx = Builder::new("t").record();
        ctx.mark("x");
        assert_eq!(
            ctx.finish(OutcomeKind::Passed, None)
                .unwrap()
                .unwrap()
                .events
                .len(),
            1
        );
        assert!(ctx.finish(OutcomeKind::Passed, None).unwrap().is_none());
    }
}

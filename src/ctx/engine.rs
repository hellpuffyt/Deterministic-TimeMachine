//! The record / replay / drive state machine behind [`Ctx`].

use std::collections::VecDeque;
use std::panic::Location;
use std::sync::MutexGuard;

use super::{Ctx, SiteCheck};
use crate::divergence::{Divergence, DivergenceReason, Request as DivRequest};
use crate::sources::Sources;
use crate::trace::{Event, Kind, Trace, Value};

pub(super) enum Engine {
    Live,
    Record(Vec<Event>),
    Replay {
        trace: Trace,
        cursor: usize,
    },
    Drive {
        decisions: VecDeque<u64>,
        events: Vec<Event>,
    },
}

pub(super) struct State {
    pub(super) sources: Sources,
    pub(super) engine: Engine,
    pub(super) divergence: Option<Divergence>,
    pub(super) finished: bool,
}

pub(super) fn site_string(loc: &Location<'_>) -> String {
    format!(
        "{}:{}:{}",
        loc.file().replace('\\', "/"),
        loc.line(),
        loc.column()
    )
}

pub(super) fn site_file(site: &str) -> &str {
    // "file:line:col" → "file" (file names may themselves contain ':' on Windows).
    let mut parts = site.rsplitn(3, ':');
    let _col = parts.next();
    let _line = parts.next();
    parts.next().unwrap_or(site)
}

impl Ctx {
    pub(super) fn lock(&self) -> MutexGuard<'_, State> {
        // A panic while holding the lock (only possible inside a user source)
        // must not make the recording unreachable.
        self.inner
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// The core of every request: record, replay or drive one input.
    #[track_caller]
    pub(super) fn interact(
        &self,
        kind: Kind,
        key: Option<String>,
        loc: &Location<'_>,
        modulus: Option<u64>,
        live: impl FnOnce(&mut Sources) -> Value,
    ) -> Value {
        let site = site_string(loc);
        let mut guard = self.lock();
        if let Some(d) = &guard.divergence {
            // A previous divergence was caught by the program; keep failing.
            let text = d.to_string();
            drop(guard);
            panic!("{text}");
        }
        let State {
            sources, engine, ..
        } = &mut *guard;
        let result = match engine {
            Engine::Live => Ok(live(sources)),
            Engine::Record(events) => {
                let value = live(sources);
                events.push(Event {
                    seq: events.len() as u64,
                    kind,
                    key,
                    value: value.clone(),
                    site,
                });
                Ok(value)
            }
            Engine::Drive { decisions, events } => {
                let value = if kind.is_decision() {
                    let raw = decisions.pop_front().unwrap_or(0);
                    Value::U64(match modulus {
                        Some(n) if n > 0 => raw % n,
                        _ => raw,
                    })
                } else {
                    live(sources)
                };
                events.push(Event {
                    seq: events.len() as u64,
                    kind,
                    key,
                    value: value.clone(),
                    site,
                });
                Ok(value)
            }
            Engine::Replay { trace, cursor } => {
                let index = *cursor;
                let request = DivRequest {
                    kind,
                    key: key.clone(),
                    site: site.clone(),
                };
                match trace.events.get(index) {
                    None => Err(self.make_divergence(
                        trace,
                        index,
                        DivergenceReason::PastEnd,
                        Some(request),
                        None,
                    )),
                    Some(expected) if expected.kind != kind || expected.key != key => Err(self
                        .make_divergence(
                            trace,
                            index,
                            DivergenceReason::DifferentRequest,
                            Some(request),
                            None,
                        )),
                    Some(expected) if !self.site_matches(&expected.site, &site) => Err(self
                        .make_divergence(
                            trace,
                            index,
                            DivergenceReason::DifferentSite,
                            Some(request),
                            None,
                        )),
                    Some(expected) => {
                        *cursor += 1;
                        Ok(expected.value.clone())
                    }
                }
            }
        };
        match result {
            Ok(v) => v,
            Err(d) => self.raise(guard, d),
        }
    }

    fn site_matches(&self, recorded: &str, actual: &str) -> bool {
        match self.inner.site_check {
            SiteCheck::Exact => recorded == actual,
            SiteCheck::File => site_file(recorded) == site_file(actual),
            SiteCheck::Off => true,
        }
    }

    pub(super) fn make_divergence(
        &self,
        trace: &Trace,
        index: usize,
        reason: DivergenceReason,
        actual: Option<DivRequest>,
        detail: Option<String>,
    ) -> Divergence {
        let start = index.saturating_sub(3);
        Divergence {
            index: index as u64,
            recorded_events: trace.events.len() as u64,
            reason,
            expected: trace.events.get(index).cloned(),
            actual,
            context: trace
                .events
                .get(start..index.min(trace.events.len()))
                .map(<[Event]>::to_vec)
                .unwrap_or_default(),
            detail,
        }
    }

    /// Stores the divergence, writes the report (environment-driven runs) and
    /// panics with the diagnostic. Divergence is reported by panicking because
    /// the program cannot continue meaningfully: its inputs no longer exist.
    #[track_caller]
    pub(super) fn raise(&self, mut guard: MutexGuard<'_, State>, d: Divergence) -> ! {
        let text = d.to_string();
        if guard.divergence.is_none() {
            guard.divergence = Some(d.clone());
        }
        drop(guard);
        if let Some(path) = &self.inner.persist.report_out {
            let report = crate::envmode::RunReport::diverged(&self.inner.test, d);
            // Best effort: failing to write the report must not hide the
            // divergence itself, which is still raised below.
            let _ = report.save(path);
        }
        panic!("{text}");
    }

    #[track_caller]
    pub(super) fn bad_value(
        &self,
        kind: Kind,
        key: Option<String>,
        loc: &Location<'_>,
        wanted: &str,
        got: &Value,
    ) -> ! {
        let guard = self.lock();
        let d = match &guard.engine {
            Engine::Replay { trace, cursor } => {
                let index = cursor.saturating_sub(1);
                self.make_divergence(
                    trace,
                    index,
                    DivergenceReason::BadValue,
                    Some(DivRequest {
                        kind,
                        key,
                        site: site_string(loc),
                    }),
                    Some(format!(
                        "expected a {wanted} value, found {}",
                        got.summary()
                    )),
                )
            }
            _ => Divergence {
                index: 0,
                recorded_events: 0,
                reason: DivergenceReason::BadValue,
                expected: None,
                actual: Some(DivRequest {
                    kind,
                    key,
                    site: site_string(loc),
                }),
                context: Vec::new(),
                detail: Some(format!(
                    "a source returned an unexpected {wanted} value: {}",
                    got.summary()
                )),
            },
        };
        self.raise(guard, d)
    }

    #[track_caller]
    pub(super) fn u64_value(
        &self,
        kind: Kind,
        key: Option<String>,
        loc: &Location<'_>,
        v: Value,
    ) -> u64 {
        match v {
            Value::U64(n) => n,
            other => self.bad_value(kind, key, loc, "u64", &other),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn site_file_strips_line_and_column() {
        assert_eq!(site_file("src/a.rs:10:5"), "src/a.rs");
        assert_eq!(site_file("C:/x/a.rs:1:2"), "C:/x/a.rs");
        assert_eq!(site_file("weird"), "weird");
    }
}

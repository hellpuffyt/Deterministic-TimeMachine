//! Replay mismatch diagnostics.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::trace::{Event, Kind};

/// Every divergence message starts with this text.
pub const DIVERGENCE_PREFIX: &str = "replay diverged at event #";

/// Why a replay stopped matching the recorded trace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DivergenceReason {
    /// The program asked for a different kind of input, or for a different
    /// key (path, variable, request, …) than the one recorded at this point.
    DifferentRequest,
    /// Same request, but issued from a different call site (only reported
    /// when site checking is enabled).
    DifferentSite,
    /// The program asked for more inputs than the trace contains.
    PastEnd,
    /// The program finished before consuming every recorded input.
    EndedEarly,
    /// The recorded value has the wrong type for this request (the trace was
    /// edited or corrupted).
    BadValue,
}

/// The input the program requested when the divergence was detected.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Request {
    /// Kind of input.
    pub kind: Kind,
    /// Key, if any.
    pub key: Option<String>,
    /// Call site.
    pub site: String,
}

impl Request {
    fn describe(&self) -> String {
        let key = self
            .key
            .as_deref()
            .map(|k| format!(" {k:?}"))
            .unwrap_or_default();
        format!("{}{}  at {}", self.kind, key, self.site)
    }
}

/// A precise description of where and how a replay diverged.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Divergence {
    /// Index of the recorded event at which the mismatch occurred.
    pub index: u64,
    /// Number of events in the recorded trace.
    pub recorded_events: u64,
    /// What went wrong.
    pub reason: DivergenceReason,
    /// The recorded event at `index`, if the trace has one.
    pub expected: Option<Event>,
    /// What the program asked for instead (absent for `EndedEarly`).
    pub actual: Option<Request>,
    /// Up to three recorded events before `index`, for orientation.
    pub context: Vec<Event>,
    /// Extra detail (e.g. which value type was expected).
    pub detail: Option<String>,
}

impl fmt::Display for Divergence {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let what = match self.reason {
            DivergenceReason::DifferentRequest => {
                "the program requested a different input than was recorded"
            }
            DivergenceReason::DifferentSite => {
                "the same input was requested from a different call site"
            }
            DivergenceReason::PastEnd => "the program requested more inputs than were recorded",
            DivergenceReason::EndedEarly => {
                "the program finished before using every recorded input"
            }
            DivergenceReason::BadValue => {
                "the recorded value does not fit the request (corrupted or edited trace?)"
            }
        };
        writeln!(
            f,
            "{DIVERGENCE_PREFIX}{} of {}: {}",
            self.index, self.recorded_events, what
        )?;
        match &self.expected {
            Some(e) => writeln!(f, "  expected: {}", e.describe())?,
            None => writeln!(f, "  expected: <end of trace>")?,
        }
        match &self.actual {
            Some(r) => writeln!(f, "  actual:   #{} {}", self.index, r.describe())?,
            None => writeln!(f, "  actual:   <program finished>")?,
        }
        if let Some(d) = &self.detail {
            writeln!(f, "  detail:   {d}")?;
        }
        if !self.context.is_empty() {
            writeln!(f, "  preceding recorded events:")?;
            for e in &self.context {
                writeln!(f, "    {}", e.describe())?;
            }
        }
        write!(
            f,
            "  hint: the code or its inputs changed since the trace was recorded; re-record it, or fix the change that made the run take a different path"
        )
    }
}

impl std::error::Error for Divergence {}

//! Environment-driven runs: how the `tm` CLI controls a program it launches.
//!
//! A program opts in by building its context with [`Builder::from_env`] (or by
//! using [`run_main`]). The CLI then sets these variables on the child:
//!
//! | variable       | meaning |
//! |----------------|---------|
//! | `TM_MODE`      | `live` (default), `record`, `replay` or `drive` |
//! | `TM_TEST`      | if set, only the context with this test name obeys the other variables |
//! | `TM_SEED`      | seed for simulated sources (`record`), default 0 |
//! | `TM_SOURCES`   | `sim` or `real` (`record`); default: what the builder chose |
//! | `TM_TRACE`     | trace to write (`record`, `drive`) or read (`replay`) |
//! | `TM_DECISIONS` | JSON array of decisions (`drive`) |
//! | `TM_REPORT`    | where to write a JSON run report (all modes but `live`) |
//!
//! Without any of them set, a program behaves exactly as it would without
//! timemachine (live mode with the sources it configured).

use std::fmt::Display;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use serde::{Deserialize, Serialize};

use crate::ctx::{Builder, Ctx, Mode, Persist};
use crate::divergence::Divergence;
use crate::harness;
use crate::trace::{OutcomeKind, Trace, TraceError};

/// Name of the mode variable.
pub const TM_MODE: &str = "TM_MODE";
/// Name of the test-filter variable.
pub const TM_TEST: &str = "TM_TEST";
/// Name of the seed variable.
pub const TM_SEED: &str = "TM_SEED";
/// Name of the sources variable.
pub const TM_SOURCES: &str = "TM_SOURCES";
/// Name of the trace-path variable.
pub const TM_TRACE: &str = "TM_TRACE";
/// Name of the decisions-path variable.
pub const TM_DECISIONS: &str = "TM_DECISIONS";
/// Name of the report-path variable.
pub const TM_REPORT: &str = "TM_REPORT";

/// Exit code of [`run_main`] when replay diverged.
pub const EXIT_DIVERGED: u8 = 3;
/// Exit code of [`run_main`] when the `TM_*` configuration is invalid.
pub const EXIT_CONFIG: u8 = 2;

/// Invalid `TM_*` configuration.
#[derive(Debug, thiserror::Error)]
pub enum EnvError {
    /// A required variable is missing.
    #[error("{var} must be set when TM_MODE={mode}")]
    Missing {
        /// Variable name.
        var: &'static str,
        /// Mode that needs it.
        mode: String,
    },
    /// A variable has an invalid value.
    #[error("invalid {var}={value:?}: {reason}")]
    Invalid {
        /// Variable name.
        var: &'static str,
        /// Its value.
        value: String,
        /// Why it is invalid.
        reason: String,
    },
    /// The trace to replay could not be loaded.
    #[error("cannot load trace {path}: {source}")]
    Trace {
        /// Trace path.
        path: PathBuf,
        /// Underlying error.
        source: TraceError,
    },
    /// The decisions file could not be loaded.
    #[error("cannot load decisions {path}: {message}")]
    Decisions {
        /// Decisions path.
        path: PathBuf,
        /// What went wrong.
        message: String,
    },
}

fn var(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

fn path_var(name: &'static str, mode: &str) -> Result<PathBuf, EnvError> {
    var(name)
        .map(PathBuf::from)
        .ok_or_else(|| EnvError::Missing {
            var: name,
            mode: mode.to_owned(),
        })
}

impl Builder {
    /// Builds a context according to the `TM_*` environment variables (see
    /// the [module docs](crate::envmode)). With none set, this is `live()`.
    pub fn from_env(mut self) -> Result<Ctx, EnvError> {
        if let Some(only) = var(TM_TEST) {
            if only != self.test_name() {
                return Ok(self.live());
            }
        }
        let mode = var(TM_MODE).unwrap_or_else(|| "live".into());
        let report_out = var(TM_REPORT).map(PathBuf::from);
        match mode.as_str() {
            "live" => Ok(self.live()),
            "record" => {
                if let Some(s) = var(TM_SOURCES) {
                    match s.as_str() {
                        "sim" => self.real_flag(false),
                        "real" => self.real_flag(true),
                        other => {
                            return Err(EnvError::Invalid {
                                var: TM_SOURCES,
                                value: other.into(),
                                reason: "expected `sim` or `real`".into(),
                            });
                        }
                    }
                }
                if let Some(seed) = var(TM_SEED) {
                    let parsed = seed.parse::<u64>().map_err(|e| EnvError::Invalid {
                        var: TM_SEED,
                        value: seed.clone(),
                        reason: e.to_string(),
                    })?;
                    self = self.seed(parsed);
                }
                self.persist = Persist {
                    trace_out: Some(path_var(TM_TRACE, &mode)?),
                    report_out,
                };
                Ok(self.record())
            }
            "replay" => {
                let path = path_var(TM_TRACE, &mode)?;
                let trace =
                    Trace::load(&path).map_err(|source| EnvError::Trace { path, source })?;
                self.persist = Persist {
                    trace_out: None,
                    report_out,
                };
                Ok(self.replay(trace))
            }
            "drive" => {
                let path = path_var(TM_DECISIONS, &mode)?;
                let decisions = load_decisions(&path)?;
                self.persist = Persist {
                    trace_out: var(TM_TRACE).map(PathBuf::from),
                    report_out,
                };
                Ok(self.drive(decisions))
            }
            other => Err(EnvError::Invalid {
                var: TM_MODE,
                value: other.into(),
                reason: "expected live, record, replay or drive".into(),
            }),
        }
    }
}

/// Reads a JSON array of decisions.
pub fn load_decisions(path: &Path) -> Result<Vec<u64>, EnvError> {
    let text = std::fs::read_to_string(path).map_err(|e| EnvError::Decisions {
        path: path.to_owned(),
        message: e.to_string(),
    })?;
    serde_json::from_str(&text).map_err(|e| EnvError::Decisions {
        path: path.to_owned(),
        message: e.to_string(),
    })
}

/// A machine-readable summary of one environment-driven run, written to
/// `TM_REPORT` for the `tm` CLI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunReport {
    /// Test name.
    pub test: String,
    /// `live`, `record`, `replay` or `drive`.
    pub mode: String,
    /// How the run ended.
    pub outcome: OutcomeKind,
    /// Failure message.
    pub message: Option<String>,
    /// For replays: whether the recorded outcome and message were reproduced.
    pub reproduced: Option<bool>,
    /// For replays: the divergence, if any.
    pub divergence: Option<Divergence>,
}

impl RunReport {
    pub(crate) fn diverged(test: &str, d: Divergence) -> RunReport {
        RunReport {
            test: test.to_owned(),
            mode: "replay".into(),
            outcome: OutcomeKind::Diverged,
            message: Some(d.to_string()),
            reproduced: Some(false),
            divergence: Some(d),
        }
    }

    /// Writes the report as JSON.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let json = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        std::fs::write(path, json)
    }

    /// Reads a report.
    pub fn load(path: &Path) -> std::io::Result<RunReport> {
        let text = std::fs::read_to_string(path)?;
        serde_json::from_str(&text).map_err(std::io::Error::other)
    }
}

fn mode_name(m: Mode) -> &'static str {
    match m {
        Mode::Live => "live",
        Mode::Record => "record",
        Mode::Replay => "replay",
        Mode::Drive => "drive",
    }
}

/// Runs `test` as a whole program under `TM_*` control and returns the exit
/// code: 0 passed, 1 failed or panicked, [`EXIT_DIVERGED`] (3) when replay
/// diverged, [`EXIT_CONFIG`] (2) for invalid `TM_*` variables.
///
/// ```no_run
/// use timemachine::{Builder, Ctx};
///
/// fn my_test(ctx: &Ctx) -> Result<(), String> {
///     let delay = ctx.gen_range(0..100);
///     if delay > 90 { Err(format!("too slow: {delay}")) } else { Ok(()) }
/// }
///
/// fn main() -> std::process::ExitCode {
///     timemachine::run_main("my_test", |b| b, my_test)
/// }
/// ```
pub fn run_main<S, F, E>(test_name: &str, setup: S, test: F) -> ExitCode
where
    S: FnOnce(Builder) -> Builder,
    F: FnOnce(&Ctx) -> Result<(), E>,
    E: Display,
{
    let ctx = match setup(Builder::new(test_name)).from_env() {
        Ok(ctx) => ctx,
        Err(e) => {
            eprintln!("timemachine: {e}");
            return ExitCode::from(EXIT_CONFIG);
        }
    };
    let recorded = recorded_outcome();
    let mode = ctx.mode();
    // run_main owns the process, so it may own the panic hook: divergence
    // panics are reported once, below, instead of also by the default hook.
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if !is_divergence_panic(info.payload()) {
            previous(info);
        }
    }));
    let out = harness::run(&ctx, test);
    let _ = std::panic::take_hook();
    let reproduced = match (mode, &recorded) {
        (Mode::Replay, Some((kind, msg))) => {
            Some(out.divergence.is_none() && out.kind == *kind && out.message == *msg)
        }
        _ => None,
    };
    if let Some(path) = var(TM_REPORT) {
        let report = RunReport {
            test: test_name.to_owned(),
            mode: mode_name(mode).into(),
            outcome: out.kind,
            message: out.message.clone(),
            reproduced,
            divergence: out.divergence.clone(),
        };
        if let Err(e) = report.save(Path::new(&path)) {
            eprintln!("timemachine: cannot write report {path}: {e}");
        }
    }
    match out.kind {
        OutcomeKind::Passed => ExitCode::SUCCESS,
        OutcomeKind::Diverged => {
            if let Some(d) = &out.divergence {
                eprintln!("{d}");
            }
            ExitCode::from(EXIT_DIVERGED)
        }
        _ => {
            if let Some(m) = &out.message {
                eprintln!("{test_name} failed: {m}");
            }
            ExitCode::FAILURE
        }
    }
}

fn is_divergence_panic(payload: &(dyn std::any::Any + Send)) -> bool {
    let text = payload
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| payload.downcast_ref::<&str>().copied())
        .unwrap_or_default();
    text.starts_with(crate::divergence::DIVERGENCE_PREFIX)
}

fn recorded_outcome() -> Option<(OutcomeKind, Option<String>)> {
    if var(TM_MODE).as_deref() != Some("replay") {
        return None;
    }
    let t = Trace::load(var(TM_TRACE)?).ok()?;
    Some((t.outcome, t.message))
}

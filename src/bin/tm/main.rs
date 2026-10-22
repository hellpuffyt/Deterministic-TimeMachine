//! `tm` — hunt for, replay, minimize and inspect timemachine traces.

#![forbid(unsafe_code)]

mod child;
mod hunt;
mod minimize;
mod replay;
mod view;

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand, ValueEnum};
use timemachine::minimize::FailureMatch;

#[derive(Parser)]
#[command(
    name = "tm",
    version,
    about = "Make flaky tests reproducible: hunt, replay, minimize and inspect timemachine traces"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Clone, Copy, ValueEnum)]
enum SourcesArg {
    /// Virtual clock, seeded RNG, in-memory files: failures reproduce from the seed alone.
    Sim,
    /// System clock, OS entropy, real files: record what really happened.
    Real,
}

#[derive(Subcommand)]
enum Command {
    /// Run a program repeatedly in record mode until it fails; save the failing trace.
    Hunt {
        /// Number of runs.
        #[arg(long, default_value_t = 100)]
        runs: u64,
        /// Seed of the first run (run i uses seed first_seed + i).
        #[arg(long, default_value_t = 0)]
        first_seed: u64,
        /// Override the sources the program configured.
        #[arg(long, value_enum)]
        sources: Option<SourcesArg>,
        /// Only the context with this test name is controlled.
        #[arg(long)]
        test: Option<String>,
        /// Directory for saved traces.
        #[arg(long, default_value = "tm-out")]
        out: PathBuf,
        /// Per-run timeout in seconds.
        #[arg(long, default_value_t = 60)]
        timeout: u64,
        /// Keep going after the first failure and report the failure rate.
        #[arg(long)]
        all: bool,
        /// Show the program's stdout/stderr.
        #[arg(long)]
        show_output: bool,
        /// The program and its arguments.
        #[arg(last = true, required = true)]
        cmd: Vec<String>,
    },
    /// Replay a trace against the program and check it reproduces the recorded outcome.
    Replay {
        /// Trace file.
        trace: PathBuf,
        /// Number of replays.
        #[arg(long, default_value_t = 1)]
        times: u64,
        /// Only the context with this test name is controlled.
        #[arg(long)]
        test: Option<String>,
        /// Per-run timeout in seconds.
        #[arg(long, default_value_t = 60)]
        timeout: u64,
        /// Hide the program's output (default: shown when replaying once).
        #[arg(long)]
        quiet: bool,
        /// The program and its arguments.
        #[arg(last = true, required = true)]
        cmd: Vec<String>,
    },
    /// Shrink a failing trace's decisions with delta debugging.
    Minimize {
        /// Failing trace file.
        trace: PathBuf,
        /// Where to write the minimized trace (default: the input path with extension .min.tmtrace).
        #[arg(long)]
        out: Option<PathBuf>,
        /// Maximum number of program runs.
        #[arg(long, default_value_t = 2000)]
        max_runs: u64,
        /// Accept any failure (default: same message up to its first ':').
        #[arg(long, conflicts_with = "exact_message")]
        any_failure: bool,
        /// Require the exact same failure message.
        #[arg(long)]
        exact_message: bool,
        /// Only the context with this test name is controlled.
        #[arg(long)]
        test: Option<String>,
        /// Per-run timeout in seconds.
        #[arg(long, default_value_t = 60)]
        timeout: u64,
        /// The program and its arguments.
        #[arg(last = true, required = true)]
        cmd: Vec<String>,
    },
    /// Print a trace's header, a summary and its events.
    Inspect {
        /// Trace file.
        trace: PathBuf,
        /// Maximum events to print (0 = all).
        #[arg(long, default_value_t = 50)]
        limit: usize,
        /// Only events of this kind (e.g. `rng.u64`, `fs.read`).
        #[arg(long)]
        kind: Option<String>,
        /// Skip checksum verification (for intentionally edited traces).
        #[arg(long)]
        no_verify: bool,
    },
    /// Show the first event where two traces differ.
    Diff {
        /// First trace.
        a: PathBuf,
        /// Second trace.
        b: PathBuf,
        /// Events of context to show before the difference.
        #[arg(long, default_value_t = 3)]
        context: usize,
    },
    /// Check that trace files are complete and valid.
    Validate {
        /// Trace files.
        #[arg(required = true)]
        traces: Vec<PathBuf>,
    },
}

/// Loads and verifies a trace, honouring `TM_MAX_TRACE_BYTES`.
pub fn load_trace(path: impl AsRef<std::path::Path>) -> anyhow::Result<timemachine::Trace> {
    load_trace_with(path, true)
}

/// Loads a trace, optionally without checksum verification, honouring
/// `TM_MAX_TRACE_BYTES`.
pub fn load_trace_with(
    path: impl AsRef<std::path::Path>,
    verify_checksum: bool,
) -> anyhow::Result<timemachine::Trace> {
    let limit = timemachine::envmode::max_trace_bytes()?;
    Ok(timemachine::Trace::load_with(path, limit, verify_checksum)?)
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.command {
        Command::Hunt {
            runs,
            first_seed,
            sources,
            test,
            out,
            timeout,
            all,
            show_output,
            cmd,
        } => {
            let sources = sources.map(|s| match s {
                SourcesArg::Sim => "sim",
                SourcesArg::Real => "real",
            });
            hunt::run(hunt::Options {
                runs,
                first_seed,
                sources,
                test,
                out,
                timeout,
                all,
                show_output,
                cmd,
            })
        }
        Command::Replay {
            trace,
            times,
            test,
            timeout,
            quiet,
            cmd,
        } => replay::run(replay::Options {
            trace,
            times,
            test,
            timeout,
            quiet,
            cmd,
        }),
        Command::Minimize {
            trace,
            out,
            max_runs,
            any_failure,
            exact_message,
            test,
            timeout,
            cmd,
        } => {
            let matching = if any_failure {
                FailureMatch::Any
            } else if exact_message {
                FailureMatch::Exact
            } else {
                FailureMatch::Signature
            };
            minimize::run(minimize::Options {
                trace,
                out,
                max_runs,
                matching,
                test,
                timeout,
                cmd,
            })
        }
        Command::Inspect {
            trace,
            limit,
            kind,
            no_verify,
        } => view::inspect(&trace, limit, kind.as_deref(), no_verify),
        Command::Diff { a, b, context } => view::diff(&a, &b, context),
        Command::Validate { traces } => view::validate(&traces),
    };
    match result {
        Ok(code) => code,
        Err(e) => {
            eprintln!("tm: error: {e:#}");
            ExitCode::from(2)
        }
    }
}

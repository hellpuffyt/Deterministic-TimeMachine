//! `tm hunt`: run a program repeatedly in record mode until it fails.

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use timemachine::Trace;
use timemachine::envmode::{TM_MODE, TM_REPORT, TM_SEED, TM_SOURCES, TM_TEST, TM_TRACE};

use crate::child::{Child, Scratch, display};

pub struct Options {
    pub runs: u64,
    pub first_seed: u64,
    pub sources: Option<&'static str>,
    pub test: Option<String>,
    pub out: PathBuf,
    pub timeout: u64,
    pub all: bool,
    pub show_output: bool,
    pub cmd: Vec<String>,
}

pub fn run(o: Options) -> Result<ExitCode> {
    let scratch = Scratch::new()?;
    let trace_path = scratch.path("run.tmtrace");
    let started = Instant::now();
    let mut failures = 0u64;
    let mut executed = 0u64;
    let mut first_saved: Option<PathBuf> = None;
    let mut inconclusive = 0u64;

    for i in 0..o.runs {
        let seed = o.first_seed.saturating_add(i);
        scratch.clear("run.tmtrace");
        let mut env = vec![
            (TM_MODE, "record".to_owned()),
            (TM_SEED, seed.to_string()),
            (TM_TRACE, display(&trace_path)),
            (TM_REPORT, display(&scratch.path("report.json"))),
        ];
        if let Some(s) = o.sources {
            env.push((TM_SOURCES, s.to_owned()));
        }
        if let Some(t) = &o.test {
            env.push((TM_TEST, t.clone()));
        }
        let exit = Child {
            cmd: &o.cmd,
            env,
            timeout: Duration::from_secs(o.timeout),
            show_output: o.show_output,
        }
        .run()?;
        executed += 1;
        if exit.success() {
            continue;
        }
        if matches!(exit, crate::child::Exit::TimedOut) {
            inconclusive += 1;
            eprintln!(
                "tm: run {} (seed {seed}) timed out after {}s and was killed; it is not counted as a failure",
                i + 1,
                o.timeout
            );
            continue;
        }
        let trace = match crate::load_trace(&trace_path) {
            Ok(t) => t,
            Err(e) => {
                inconclusive += 1;
                eprintln!(
                    "tm: run {} (seed {seed}) ended with {} but left no usable trace ({e}); does the program build its context with Builder::from_env or run_main{}?",
                    i + 1,
                    exit.describe(),
                    if o.test.is_some() {
                        ", and does --test match its name"
                    } else {
                        ""
                    }
                );
                continue;
            }
        };
        if !trace.outcome.is_failure() && trace.outcome != timemachine::OutcomeKind::Unknown {
            continue;
        }
        failures += 1;
        if first_saved.is_none() {
            std::fs::create_dir_all(&o.out)
                .with_context(|| format!("cannot create {}", o.out.display()))?;
            // Real-sources recordings have no seed; name them by run number.
            let (label, file) = trace_name(&trace, i + 1);
            let dest = o.out.join(file);
            trace
                .save(&dest)
                .with_context(|| format!("cannot save {}", dest.display()))?;
            println!(
                "found a failure on run {} ({label}, {}): {}{}",
                i + 1,
                exit.describe(),
                trace.outcome,
                trace
                    .message
                    .as_deref()
                    .map(|m| format!(": {m}"))
                    .unwrap_or_default()
            );
            println!("saved {} ({} events)", dest.display(), trace.events.len());
            println!(
                "reproduce with: tm replay {} -- {}",
                dest.display(),
                o.cmd.join(" ")
            );
            first_saved = Some(dest);
        }
        if !o.all {
            break;
        }
    }

    let secs = started.elapsed().as_secs_f64();
    if o.all {
        let pct = if executed > 0 {
            failures as f64 * 100.0 / executed as f64
        } else {
            0.0
        };
        println!("{executed} runs in {secs:.1}s: {failures} failed ({pct:.1}%)");
    } else if first_saved.is_none() {
        println!("no failure in {executed} runs ({secs:.1}s)");
    }
    if inconclusive > 0 && failures == 0 {
        println!("{inconclusive} run(s) could not be evaluated (see messages above)");
        return Ok(ExitCode::from(2));
    }
    Ok(if first_saved.is_some() {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    })
}

/// Label for messages and file name for a saved failing trace. Simulated
/// recordings are named by seed; real-sources recordings have no seed and
/// are named by run number.
fn trace_name(trace: &Trace, run: u64) -> (String, String) {
    let test = sanitize(&trace.header.test);
    match trace.header.seed {
        Some(s) => (format!("seed {s}"), format!("{test}-seed{s}.tmtrace")),
        None => (
            format!("{} sources", trace.header.sources),
            format!("{test}-run{run}.tmtrace"),
        ),
    }
}

fn sanitize(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use timemachine::OutcomeKind;
    use timemachine::trace::Header;

    fn trace(seed: Option<u64>, sources: &str) -> Trace {
        Trace {
            header: Header::new("my test/1", seed, sources),
            events: vec![],
            outcome: OutcomeKind::Failed,
            message: None,
        }
    }

    #[test]
    fn seeded_traces_are_named_by_seed() {
        assert_eq!(
            trace_name(&trace(Some(7), "sim"), 3),
            ("seed 7".into(), "my_test_1-seed7.tmtrace".into())
        );
    }

    #[test]
    fn real_sources_traces_are_named_by_run_not_by_a_fake_seed() {
        // Regression: real-sources failures used to be saved as "-seed0".
        assert_eq!(
            trace_name(&trace(None, "real+custom"), 2),
            (
                "real+custom sources".into(),
                "my_test_1-run2.tmtrace".into()
            )
        );
    }
}

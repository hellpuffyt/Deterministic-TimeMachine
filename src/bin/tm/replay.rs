//! `tm replay`: replay a trace against a program and check the outcome.

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use anyhow::{Context, Result};
use timemachine::envmode::{TM_MODE, TM_REPORT, TM_TEST, TM_TRACE};
use timemachine::{RunReport, Trace};

use crate::child::{Child, Scratch, display};

pub struct Options {
    pub trace: PathBuf,
    pub times: u64,
    pub test: Option<String>,
    pub timeout: u64,
    pub quiet: bool,
    pub cmd: Vec<String>,
}

pub fn run(o: Options) -> Result<ExitCode> {
    let recorded =
        Trace::load(&o.trace).with_context(|| format!("cannot use {}", o.trace.display()))?;
    let trace_abs = std::fs::canonicalize(&o.trace)
        .with_context(|| format!("cannot resolve {}", o.trace.display()))?;
    let expected = format!(
        "{}{}",
        recorded.outcome,
        recorded
            .message
            .as_deref()
            .map(|m| format!(": {m}"))
            .unwrap_or_default()
    );
    let scratch = Scratch::new()?;
    let report_path = scratch.path("report.json");
    let (mut reproduced, mut diverged, mut other) = (0u64, 0u64, 0u64);

    for i in 0..o.times {
        scratch.clear("report.json");
        let mut env = vec![
            (TM_MODE, "replay".to_owned()),
            (TM_TRACE, display(&trace_abs)),
            (TM_REPORT, display(&report_path)),
        ];
        if let Some(t) = &o.test {
            env.push((TM_TEST, t.clone()));
        }
        let show = !o.quiet && o.times == 1;
        let exit = Child {
            cmd: &o.cmd,
            env,
            timeout: Duration::from_secs(o.timeout),
            show_output: show,
        }
        .run()?;
        match RunReport::load(&report_path) {
            Ok(report) if report.divergence.is_some() => {
                diverged += 1;
                if diverged == 1 {
                    println!("run {}: DIVERGED", i + 1);
                    // When the program's output is shown it already printed
                    // the diagnostic.
                    if let (Some(d), false) = (&report.divergence, show) {
                        println!("{d}");
                    }
                }
            }
            Ok(report) if report.reproduced == Some(true) => reproduced += 1,
            Ok(report) => {
                other += 1;
                if other == 1 {
                    println!(
                        "run {}: did not reproduce: got {}{} (recorded {expected})",
                        i + 1,
                        report.outcome,
                        report
                            .message
                            .as_deref()
                            .map(|m| format!(": {m}"))
                            .unwrap_or_default()
                    );
                }
            }
            Err(_) => {
                // No report: the program does not use run_main. Fall back to
                // the exit status, which can only confirm pass vs fail.
                let failed = !exit.success();
                if failed == recorded.outcome.is_failure() {
                    reproduced += 1;
                } else {
                    other += 1;
                    println!("run {}: {} (recorded {expected})", i + 1, exit.describe());
                }
            }
        }
    }

    println!(
        "replayed {} time(s): {reproduced} reproduced the recorded outcome ({expected}), {diverged} diverged, {other} differed",
        o.times
    );
    Ok(if diverged > 0 {
        ExitCode::from(3)
    } else if other > 0 {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    })
}

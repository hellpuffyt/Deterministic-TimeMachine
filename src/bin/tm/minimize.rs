//! `tm minimize`: shrink a failing trace by re-running the program with
//! fewer/smaller decisions.

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use timemachine::Trace;
use timemachine::envmode::{TM_DECISIONS, TM_MODE, TM_REPORT, TM_TEST, TM_TRACE};
use timemachine::minimize::{FailureMatch, shrink_decisions};

use crate::child::{Child, Scratch, display};

pub struct Options {
    pub trace: PathBuf,
    pub out: Option<PathBuf>,
    pub max_runs: u64,
    pub matching: FailureMatch,
    pub test: Option<String>,
    pub timeout: u64,
    pub cmd: Vec<String>,
}

struct Runner<'a> {
    o: &'a Options,
    scratch: Scratch,
    runs: u64,
    target: Option<String>,
}

impl Runner<'_> {
    /// Runs the program driven by `decisions`; returns its trace if it failed
    /// the way we are looking for.
    fn attempt(&mut self, decisions: &[u64]) -> Result<Option<Trace>> {
        self.runs += 1;
        let decisions_path = self.scratch.path("decisions.json");
        std::fs::write(&decisions_path, serde_json::to_string(decisions)?)?;
        self.scratch.clear("candidate.tmtrace");
        let mut env = vec![
            (TM_MODE, "drive".to_owned()),
            (TM_DECISIONS, display(&decisions_path)),
            (TM_TRACE, display(&self.scratch.path("candidate.tmtrace"))),
            (TM_REPORT, display(&self.scratch.path("report.json"))),
        ];
        if let Some(t) = &self.o.test {
            env.push((TM_TEST, t.clone()));
        }
        let exit = Child {
            cmd: &self.o.cmd,
            env,
            timeout: Duration::from_secs(self.o.timeout),
            show_output: false,
        }
        .run()?;
        if exit.success() {
            return Ok(None);
        }
        let Ok(trace) = Trace::load(self.scratch.path("candidate.tmtrace")) else {
            return Ok(None);
        };
        let same = self
            .o
            .matching
            .matches(self.target.as_deref(), trace.message.as_deref());
        Ok((trace.outcome.is_failure() && same).then_some(trace))
    }
}

pub fn run(o: Options) -> Result<ExitCode> {
    let original =
        Trace::load(&o.trace).with_context(|| format!("cannot use {}", o.trace.display()))?;
    if !original.outcome.is_failure() {
        bail!(
            "{} records a {} run; only failing traces can be minimized",
            o.trace.display(),
            original.outcome
        );
    }
    let decisions = original.decisions();
    let started = Instant::now();
    let mut runner = Runner {
        o: &o,
        scratch: Scratch::new()?,
        runs: 0,
        target: original.message.clone(),
    };
    if runner.attempt(&decisions)?.is_none() {
        bail!(
            "driving the original {} decisions did not reproduce the failure; the program's non-random inputs are probably not deterministic (record with simulated sources)",
            decisions.len()
        );
    }

    let mut error = None;
    let max = o.max_runs;
    let best = shrink_decisions(decisions.clone(), |candidate| {
        if error.is_some() || runner.runs >= max {
            return false;
        }
        match runner.attempt(candidate) {
            Ok(found) => found.is_some(),
            Err(e) => {
                error = Some(e);
                false
            }
        }
    });
    if let Some(e) = error {
        return Err(e);
    }
    let Some(minimized) = runner.attempt(&best)? else {
        bail!(
            "the minimized decisions stopped failing on the confirmation run (the program may be nondeterministic)"
        );
    };
    let out = o
        .out
        .clone()
        .unwrap_or_else(|| o.trace.with_extension("min.tmtrace"));
    minimized
        .save(&out)
        .with_context(|| format!("cannot save {}", out.display()))?;
    println!(
        "decisions: {} -> {}   events: {} -> {}   program runs: {} ({:.1}s){}",
        decisions.len(),
        best.len(),
        original.events.len(),
        minimized.events.len(),
        runner.runs,
        started.elapsed().as_secs_f64(),
        if runner.runs >= max {
            "  [run budget exhausted; result may not be minimal]"
        } else {
            ""
        }
    );
    println!("minimized decisions: {best:?}");
    println!("saved {}", out.display());
    Ok(ExitCode::SUCCESS)
}

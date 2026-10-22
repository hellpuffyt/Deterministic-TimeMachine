//! A deterministic cooperative scheduler for exploring interleavings.
//!
//! Model concurrent code as tasks that run in small steps. At every step the
//! scheduler picks the next runnable task with [`Ctx::choose`], so the order
//! is recorded, replayable and minimizable. This does not control OS threads;
//! it controls the order of *your* steps (see `docs/UNSUPPORTED.md`).

use crate::ctx::Ctx;

/// What a task's step reports back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// The task has more work.
    Yield,
    /// The task is finished.
    Done,
}

/// A named task: a closure called once per step.
pub struct Task<'a> {
    name: String,
    step: Box<dyn FnMut(&Ctx) -> Step + 'a>,
}

impl<'a> Task<'a> {
    /// Creates a task.
    pub fn new(name: impl Into<String>, step: impl FnMut(&Ctx) -> Step + 'a) -> Self {
        Task {
            name: name.into(),
            step: Box::new(step),
        }
    }
}

/// The order in which steps ran, and whether the step limit was hit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Schedule {
    /// Task names in execution order, one entry per step.
    pub steps: Vec<String>,
    /// `true` if `max_steps` was reached with tasks still runnable.
    pub truncated: bool,
}

/// Runs `tasks` to completion (or `max_steps`), choosing which runnable task
/// steps next with `ctx.choose`. Recorded choices carry the call site of
/// `run` itself.
#[track_caller]
pub fn run(ctx: &Ctx, mut tasks: Vec<Task<'_>>, max_steps: usize) -> Schedule {
    let mut done = vec![false; tasks.len()];
    let mut steps = Vec::new();
    loop {
        let runnable: Vec<usize> = (0..tasks.len()).filter(|&i| !done[i]).collect();
        if runnable.is_empty() {
            return Schedule {
                steps,
                truncated: false,
            };
        }
        if steps.len() >= max_steps {
            return Schedule {
                steps,
                truncated: true,
            };
        }
        let pick = if runnable.len() == 1 {
            runnable[0]
        } else {
            runnable[ctx.choose(runnable.len())]
        };
        steps.push(tasks[pick].name.clone());
        if (tasks[pick].step)(ctx) == Step::Done {
            done[pick] = true;
        }
    }
}

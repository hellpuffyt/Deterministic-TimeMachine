//! An ordering bug: workers increment a shared counter with a separate read
//! and write step. If two reads happen before the matching writes, an update
//! is lost. The deterministic scheduler explores interleavings; every choice
//! is recorded, so a failing interleaving replays exactly and the minimizer
//! can shrink it to the single choice that matters.
//!
//! ```text
//! tm hunt --sources sim -- target/debug/examples/lost_update
//! tm minimize tm-out/lost_update-seed<N>.tmtrace -- target/debug/examples/lost_update
//! ```

use std::cell::Cell;
use std::process::ExitCode;

use timemachine::Ctx;
use timemachine::sched::{self, Step, Task};

const WORKERS: usize = 4;

fn counter_reaches_worker_count(ctx: &Ctx) -> Result<(), String> {
    let counter = Cell::new(0u32);
    let tasks: Vec<Task<'_>> = (0..WORKERS)
        .map(|w| {
            let counter = &counter;
            let mut local: Option<u32> = None;
            Task::new(format!("worker-{w}"), move |_ctx: &Ctx| match local {
                None => {
                    local = Some(counter.get()); // read
                    Step::Yield
                }
                Some(v) => {
                    counter.set(v + 1); // write
                    Step::Done
                }
            })
        })
        .collect();
    let schedule = sched::run(ctx, tasks, 1_000);
    let expected = WORKERS as u32;
    if counter.get() != expected {
        return Err(format!(
            "lost update: counter is {} after {expected} increments (schedule: {})",
            counter.get(),
            schedule.steps.join(" ")
        ));
    }
    Ok(())
}

fn main() -> ExitCode {
    timemachine::run_main("lost_update", |b| b, counter_reaches_worker_count)
}

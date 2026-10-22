//! Scale check for large recordings: records `N` events, writes the trace,
//! validates it and replays it, one phase per process so peak memory can be
//! measured per phase with an external tool (`/usr/bin/time -l` on macOS,
//! `/usr/bin/time -v` on Linux).
//!
//! ```text
//! cargo build --release --example trace_scale
//! /usr/bin/time -l target/release/examples/trace_scale record   1000000 big.tmtrace
//! /usr/bin/time -l target/release/examples/trace_scale validate 1000000 big.tmtrace
//! /usr/bin/time -l target/release/examples/trace_scale replay   1000000 big.tmtrace
//! ```
//!
//! Results measured on this repository are in `docs/DETERMINISM.md`.

use std::process::ExitCode;
use std::time::{Duration, Instant};

use timemachine::trace::{DEFAULT_MAX_TRACE_BYTES, OutcomeKind};
use timemachine::{Builder, Ctx, Trace, harness};

/// The recorded program: a mix of clock reads, sleeps and random draws.
fn workload(ctx: &Ctx, n: u64) -> Result<(), String> {
    let mut acc = 0u64;
    for i in 0..n {
        match i % 3 {
            0 => acc = acc.wrapping_add(ctx.next_u64()),
            1 => acc ^= ctx.now_nanos(),
            _ => ctx.sleep(Duration::from_nanos(i)),
        }
    }
    if acc == 42 {
        return Err("unreachable in practice".into());
    }
    Ok(())
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    let (Some(phase), Some(n), Some(path)) = (args.get(1), args.get(2), args.get(3)) else {
        eprintln!("usage: trace_scale record|validate|replay <events> <trace-path>");
        return ExitCode::from(2);
    };
    let Ok(n) = n.parse::<u64>() else {
        eprintln!("<events> must be an integer");
        return ExitCode::from(2);
    };
    // Large recordings are opted into explicitly by raising the load cap.
    let limit = DEFAULT_MAX_TRACE_BYTES.max(n.saturating_mul(256));
    let start = Instant::now();
    let result: Result<String, String> = match phase.as_str() {
        "record" => {
            let ctx = Builder::new("scale").seed(1).record();
            let out = harness::run(&ctx, |c| workload(c, n));
            match out.trace {
                Some(t) => t
                    .save(path)
                    .map(|()| format!("recorded {} events", t.events.len()))
                    .map_err(|e| e.to_string()),
                None => Err("no trace".into()),
            }
        }
        "validate" => Trace::load_with_limit(path, limit)
            .map(|t| format!("valid trace with {} events", t.events.len()))
            .map_err(|e| e.to_string()),
        "replay" => match Trace::load_with_limit(path, limit) {
            Ok(t) => {
                let events = t.events.len();
                let r = harness::replay_owned(t, |b| b, |c| workload(c, n));
                if r.reproduced && r.outcome == OutcomeKind::Passed {
                    Ok(format!("replayed {events} events, reproduced"))
                } else {
                    Err(format!("replay did not reproduce: {:?}", r.message))
                }
            }
            Err(e) => Err(e.to_string()),
        },
        other => Err(format!("unknown phase {other:?}")),
    };
    match result {
        Ok(msg) => {
            println!("{phase}: {msg} in {:.2}s", start.elapsed().as_secs_f64());
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("{phase}: {e}");
            ExitCode::FAILURE
        }
    }
}

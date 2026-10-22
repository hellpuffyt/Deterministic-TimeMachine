//! Large and endless inputs: trace loading is bounded by a size limit, so a
//! huge file, a FIFO or `/dev/zero` cannot exhaust memory.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use timemachine::trace::DEFAULT_MAX_TRACE_BYTES;
use timemachine::{Builder, Ctx, Trace, TraceError, harness};

fn small_trace() -> Trace {
    let ctx = Builder::new("big").seed(9).record();
    harness::run(&ctx, |ctx: &Ctx| -> Result<(), String> {
        for _ in 0..100 {
            ctx.next_u64();
        }
        Ok(())
    })
    .trace
    .unwrap()
}

#[test]
fn a_file_over_the_default_limit_is_refused_before_it_is_read() {
    // A sparse file: cheap to create, but reading it fully (as the loader
    // used to) would allocate more than a gigabyte.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("huge.tmtrace");
    let f = std::fs::File::create(&path).unwrap();
    f.set_len(DEFAULT_MAX_TRACE_BYTES + 1).unwrap();
    drop(f);
    let err = Trace::load(&path).unwrap_err();
    assert!(
        matches!(err, TraceError::TooLarge { limit, size: Some(s) }
            if limit == DEFAULT_MAX_TRACE_BYTES && s == DEFAULT_MAX_TRACE_BYTES + 1),
        "{err}"
    );
    assert!(err.to_string().contains("TM_MAX_TRACE_BYTES"), "{err}");
}

#[test]
fn the_limit_is_inclusive_and_configurable() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("t.tmtrace");
    let t = small_trace();
    t.save(&path).unwrap();
    let len = std::fs::metadata(&path).unwrap().len();
    assert_eq!(Trace::load_with_limit(&path, len).unwrap(), t);
    assert!(matches!(
        Trace::load_with_limit(&path, len - 1),
        Err(TraceError::TooLarge { size: Some(s), .. }) if s == len
    ));
}

#[cfg(unix)]
#[test]
fn an_endless_input_stops_at_the_limit() {
    // `/dev/zero` reports length 0 and never ends: the size check up front
    // cannot catch it, the bounded reader must.
    let err = Trace::load_with_limit("/dev/zero", 64 * 1024).unwrap_err();
    assert!(
        matches!(err, TraceError::TooLarge { size: None, .. }),
        "{err}"
    );
}

#[test]
fn a_trace_of_many_events_round_trips_through_the_streaming_reader() {
    let ctx = Builder::new("many").seed(4).record();
    let out = harness::run(&ctx, |ctx: &Ctx| -> Result<(), String> {
        for i in 0..50_000u64 {
            if i % 2 == 0 {
                ctx.next_u64();
            } else {
                ctx.now_nanos();
            }
        }
        Ok(())
    });
    let t = out.trace.unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("many.tmtrace");
    t.save(&path).unwrap();
    let loaded = Trace::load(&path).unwrap();
    assert_eq!(loaded.events.len(), 50_000);
    assert_eq!(loaded, t);
    // Replay without copying the trace.
    let r = harness::replay_owned(
        loaded,
        |b| b,
        |ctx: &Ctx| -> Result<(), String> {
            for i in 0..50_000u64 {
                if i % 2 == 0 {
                    ctx.next_u64();
                } else {
                    ctx.now_nanos();
                }
            }
            Ok(())
        },
    );
    assert!(r.reproduced, "{:?}", r.divergence);
}

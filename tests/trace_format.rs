//! Malformed-input tests: corrupted, truncated and version-mismatched traces
//! are rejected with actionable errors, never misread.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use proptest::prelude::*;
use timemachine::{Builder, Ctx, Trace, TraceError, harness};

fn sample_text() -> String {
    let ctx = Builder::new("fmt").seed(3).record();
    let out = harness::run(&ctx, |ctx: &Ctx| -> Result<(), String> {
        ctx.next_u64();
        let _ = ctx.read("missing.txt");
        ctx.mark("done");
        Err("planned failure: x".into())
    });
    out.trace.unwrap().to_text()
}

fn lines(text: &str) -> Vec<String> {
    text.lines().map(str::to_owned).collect()
}

#[test]
fn valid_trace_parses() {
    let t = Trace::parse(&sample_text()).unwrap();
    assert_eq!(t.events.len(), 3);
    assert_eq!(t.message.as_deref(), Some("planned failure: x"));
}

#[test]
fn empty_input() {
    assert!(matches!(Trace::parse(""), Err(TraceError::Empty)));
    assert!(matches!(Trace::parse("\n \n"), Err(TraceError::Empty)));
}

#[test]
fn not_a_trace() {
    assert!(matches!(
        Trace::parse("hello"),
        Err(TraceError::Malformed { line: 1, .. })
    ));
    assert!(matches!(
        Trace::parse("{\"something\":1}"),
        Err(TraceError::MissingHeader)
    ));
}

#[test]
fn wrong_format_and_version() {
    let text = sample_text();
    let other = text.replacen("timemachine-trace", "other-trace", 1);
    assert!(
        matches!(Trace::parse(&other), Err(TraceError::WrongFormat { found }) if found == "other-trace")
    );
    let v2 = text.replacen("\"version\":1", "\"version\":2", 1);
    let err = Trace::parse(&v2).unwrap_err();
    assert!(matches!(
        err,
        TraceError::UnsupportedVersion {
            found: 2,
            supported: 1
        }
    ));
    assert!(err.to_string().contains("re-record"), "{err}");
}

#[test]
fn truncation_is_detected() {
    let l = lines(&sample_text());
    let cut = l[..l.len() - 1].join("\n");
    let err = Trace::parse(&cut).unwrap_err();
    assert!(matches!(err, TraceError::Truncated { events: 3 }));
    assert!(err.to_string().contains("crashed or was killed"));
    // Cut in the middle of a line.
    let text = sample_text();
    let half = &text[..text.len() * 2 / 3];
    assert!(Trace::parse(half).is_err());
}

#[test]
fn reordered_or_removed_events_are_detected() {
    let mut l = lines(&sample_text());
    l.swap(1, 2);
    assert!(matches!(
        Trace::parse(&l.join("\n")),
        Err(TraceError::BadSequence {
            line: 2,
            expected: 0,
            found: 1
        })
    ));
    let mut l = lines(&sample_text());
    l.remove(2);
    assert!(matches!(
        Trace::parse(&l.join("\n")),
        Err(TraceError::BadSequence { .. })
    ));
}

#[test]
fn edits_break_the_checksum_unless_verification_is_disabled() {
    let text = sample_text().replacen("missing.txt", "missinG.txt", 1);
    let err = Trace::parse(&text).unwrap_err();
    assert!(matches!(err, TraceError::ChecksumMismatch { .. }));
    let t = Trace::parse_with(&text, false).unwrap();
    assert_eq!(t.events[1].key.as_deref(), Some("missinG.txt"));
}

#[test]
fn trailing_data_duplicate_header_and_count_mismatch() {
    let text = sample_text();
    let l = lines(&text);
    let trailing = format!("{text}{}\n", l[1]);
    assert!(matches!(
        Trace::parse(&trailing),
        Err(TraceError::TrailingData { .. })
    ));
    let dup = [l[0].clone(), l[0].clone()]
        .iter()
        .chain(&l[1..])
        .cloned()
        .collect::<Vec<_>>()
        .join("\n");
    assert!(matches!(
        Trace::parse(&dup),
        Err(TraceError::DuplicateHeader { line: 2 })
    ));
    let count = text.replacen("\"events\":3", "\"events\":4", 1);
    assert!(matches!(
        Trace::parse(&count),
        Err(TraceError::CountMismatch {
            declared: 4,
            actual: 3
        })
    ));
}

#[test]
fn save_is_atomic_and_load_round_trips() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("x.tmtrace");
    let t = Trace::parse(&sample_text()).unwrap();
    t.save(&path).unwrap();
    assert_eq!(Trace::load(&path).unwrap(), t);
    let leftovers: Vec<_> = std::fs::read_dir(dir.path()).unwrap().collect();
    assert_eq!(leftovers.len(), 1, "no temporary file may remain");
    assert!(matches!(
        Trace::load(dir.path().join("absent")),
        Err(TraceError::Io(_))
    ));
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// Arbitrary byte corruption never panics and never yields a trace whose
    /// events silently differ from the original.
    #[test]
    fn corruption_is_rejected_or_harmless(pos in 0usize..10_000, byte in any::<u8>()) {
        let text = sample_text();
        let mut bytes = text.clone().into_bytes();
        let i = pos % bytes.len();
        bytes[i] = byte;
        if let Ok(corrupted) = String::from_utf8(bytes) {
            if let Ok(t) = Trace::parse(&corrupted) {
                let original = Trace::parse(&text).unwrap();
                prop_assert_eq!(t.events, original.events);
            }
        }
    }
}

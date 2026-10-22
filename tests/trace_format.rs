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

#[test]
fn invalid_utf8_in_a_file_is_a_malformed_line_not_an_io_error() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bad.tmtrace");
    let mut bytes = sample_text().into_bytes();
    let second_line = bytes.iter().position(|b| *b == b'\n').unwrap() + 1;
    bytes[second_line + 3] = 0xff;
    std::fs::write(&path, &bytes).unwrap();
    let err = Trace::load(&path).unwrap_err();
    assert!(
        matches!(err, TraceError::Malformed { line: 2, .. }),
        "{err}"
    );
    assert!(err.to_string().contains("UTF-8"), "{err}");
}

#[test]
fn crlf_line_endings_are_accepted_and_keep_the_checksum_valid() {
    // `git config core.autocrlf true` checks fixtures out with CRLF.
    let text = sample_text();
    let crlf = text.replace('\n', "\r\n");
    let original = Trace::parse(&text).unwrap();
    assert_eq!(Trace::parse(&crlf).unwrap(), original);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("crlf.tmtrace");
    std::fs::write(&path, crlf).unwrap();
    assert_eq!(Trace::load(&path).unwrap(), original);
}

#[test]
fn header_and_footer_edits_are_not_covered_by_the_checksum_but_fail_replay() {
    // The checksum covers the event lines only (documented in
    // docs/DETERMINISM.md). An edited footer still parses, but replay
    // compares the program's real outcome with the edited one and reports
    // that it was not reproduced.
    let text = sample_text();
    let edited = text.replace("planned failure: x", "planned failure: y");
    let t = Trace::parse(&edited).unwrap();
    let r = harness::replay(
        &t,
        |b| b,
        |ctx: &Ctx| -> Result<(), String> {
            ctx.next_u64();
            let _ = ctx.read("missing.txt");
            ctx.mark("done");
            Err("planned failure: x".into())
        },
    );
    assert!(r.divergence.is_none());
    assert!(
        !r.reproduced,
        "an edited outcome must not count as reproduced"
    );
}

#[test]
fn save_streams_and_leaves_no_partial_file_on_failure() {
    let dir = tempfile::tempdir().unwrap();
    let t = Trace::parse(&sample_text()).unwrap();
    // The destination's parent does not exist: the save fails cleanly.
    let missing = dir.path().join("no-such-dir").join("x.tmtrace");
    assert!(matches!(t.save(&missing), Err(TraceError::Io(_))));
    // Saving over an existing trace replaces it atomically.
    let path = dir.path().join("x.tmtrace");
    std::fs::write(&path, "old contents").unwrap();
    t.save(&path).unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), t.to_text());
    let names: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(names.len(), 1, "no temporary file may remain: {names:?}");
}

/// Parses the same bytes from memory and from a file; the streaming file
/// reader must agree with the in-memory parser.
fn parse_both(bytes: &[u8]) -> (Option<Result<Trace, String>>, Result<Trace, String>) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("t.tmtrace");
    std::fs::write(&path, bytes).unwrap();
    let from_file = Trace::load(&path).map_err(|e| e.to_string());
    let from_text = std::str::from_utf8(bytes)
        .ok()
        .map(|s| Trace::parse(s).map_err(|e| e.to_string()));
    (from_text, from_file)
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
        if let Ok(corrupted) = String::from_utf8(bytes.clone()) {
            if let Ok(t) = Trace::parse(&corrupted) {
                let original = Trace::parse(&text).unwrap();
                prop_assert_eq!(t.events, original.events);
            }
        }
        // The file reader sees the same bytes (including invalid UTF-8) and
        // must reach the same verdict as the in-memory parser.
        let (from_text, from_file) = parse_both(&bytes);
        match from_text {
            Some(Ok(t)) => prop_assert_eq!(from_file, Ok(t)),
            Some(Err(_)) => prop_assert!(from_file.is_err()),
            None => prop_assert!(from_file.is_err(), "invalid UTF-8 must be rejected"),
        }
    }

    /// Cutting a trace at any byte is detected, by both readers, unless only
    /// the final newline was lost.
    #[test]
    fn truncation_at_any_byte_is_rejected(cut in 0usize..10_000) {
        let text = sample_text();
        let cut = cut % text.len();
        let prefix = &text.as_bytes()[..cut];
        let (from_text, from_file) = parse_both(prefix);
        let complete = cut + 1 == text.len();
        prop_assert_eq!(from_file.is_ok(), complete);
        if let Some(r) = from_text {
            prop_assert_eq!(r.is_ok(), complete);
        }
    }

    /// Swapping two event lines is always detected.
    #[test]
    fn swapping_any_two_events_is_rejected(a in 1usize..4, b in 1usize..4) {
        prop_assume!(a != b);
        let mut ls = lines(&sample_text());
        ls.swap(a, b);
        let swapped = ls.join("\n");
        prop_assert!(
            matches!(Trace::parse(&swapped), Err(TraceError::BadSequence { .. })),
            "swap {} <-> {} was accepted", a, b
        );
    }
}

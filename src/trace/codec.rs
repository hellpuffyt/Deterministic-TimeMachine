//! Serializing and validating trace files.

use std::path::Path;

use super::{FORMAT, Footer, Line, Trace, TraceError, VERSION, Value};

/// FNV-1a 64-bit hash, used for trace checksums and content keys.
pub fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        hash ^= u64::from(*b);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

struct Checksum(u64);

impl Checksum {
    fn new() -> Self {
        Checksum(0xcbf2_9ce4_8422_2325)
    }
    fn update(&mut self, line: &str) {
        for b in line.as_bytes().iter().chain(std::iter::once(&b'\n')) {
            self.0 ^= u64::from(*b);
            self.0 = self.0.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    fn hex(&self) -> String {
        format!("{:016x}", self.0)
    }
}

fn to_line(line: &Line) -> String {
    // Serializing these plain data types cannot fail (no maps with non-string
    // keys, no custom serializers), so an error here is a programming bug.
    serde_json::to_string(line)
        .unwrap_or_else(|e| format!("{{\"serialize_error\":{:?}}}", e.to_string()))
}

impl Trace {
    /// Serializes the trace, computing the footer.
    pub fn to_text(&self) -> String {
        let mut out = to_line(&Line::Header(self.header.clone()));
        out.push('\n');
        let mut sum = Checksum::new();
        for event in &self.events {
            let line = to_line(&Line::Event(event.clone()));
            sum.update(&line);
            out.push_str(&line);
            out.push('\n');
        }
        let footer = Footer {
            events: self.events.len() as u64,
            outcome: self.outcome,
            message: self.message.clone(),
            checksum: sum.hex(),
        };
        out.push_str(&to_line(&Line::End(footer)));
        out.push('\n');
        out
    }

    /// Writes the trace to `path` (via a temporary file and rename, so readers
    /// never observe a half-written trace).
    pub fn save(&self, path: impl AsRef<Path>) -> Result<(), TraceError> {
        let path = path.as_ref();
        let tmp = path.with_extension("tmtrace.partial");
        std::fs::write(&tmp, self.to_text())?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }

    /// Reads and fully validates a trace file.
    pub fn load(path: impl AsRef<Path>) -> Result<Trace, TraceError> {
        Trace::parse(&std::fs::read_to_string(path)?)
    }

    /// Parses and fully validates trace text (including the checksum).
    pub fn parse(text: &str) -> Result<Trace, TraceError> {
        Trace::parse_with(text, true)
    }

    /// Parses trace text, optionally skipping checksum verification (for
    /// traces that were edited by hand on purpose).
    pub fn parse_with(text: &str, verify_checksum: bool) -> Result<Trace, TraceError> {
        let mut lines = text
            .lines()
            .enumerate()
            .filter(|(_, l)| !l.trim().is_empty());
        let (_, first) = lines.next().ok_or(TraceError::Empty)?;
        let header = match serde_json::from_str::<Line>(first) {
            Ok(Line::Header(h)) => h,
            Ok(_) => return Err(TraceError::MissingHeader),
            Err(e) => {
                // Distinguish "not ours" from "ours but newer/older".
                if let Ok(raw) = serde_json::from_str::<serde_json::Value>(first) {
                    if let Some(h) = raw.get("header") {
                        let format = h.get("format").and_then(|f| f.as_str()).unwrap_or_default();
                        if format != FORMAT {
                            return Err(TraceError::WrongFormat {
                                found: format.into(),
                            });
                        }
                        if let Some(v) = h.get("version").and_then(|v| v.as_u64()) {
                            if v != u64::from(VERSION) {
                                return Err(TraceError::UnsupportedVersion {
                                    found: u32::try_from(v).unwrap_or(u32::MAX),
                                    supported: VERSION,
                                });
                            }
                        }
                    }
                    return Err(TraceError::MissingHeader);
                }
                return Err(TraceError::Malformed {
                    line: 1,
                    message: e.to_string(),
                });
            }
        };
        if header.format != FORMAT {
            return Err(TraceError::WrongFormat {
                found: header.format,
            });
        }
        if header.version != VERSION {
            return Err(TraceError::UnsupportedVersion {
                found: header.version,
                supported: VERSION,
            });
        }

        let mut events = Vec::new();
        let mut sum = Checksum::new();
        let mut footer = None;
        for (idx, line) in lines {
            let lineno = idx + 1;
            if footer.is_some() {
                return Err(TraceError::TrailingData { line: lineno });
            }
            match serde_json::from_str::<Line>(line) {
                Ok(Line::Event(event)) => {
                    let expected = events.len() as u64;
                    if event.seq != expected {
                        return Err(TraceError::BadSequence {
                            line: lineno,
                            expected,
                            found: event.seq,
                        });
                    }
                    sum.update(line);
                    events.push(event);
                }
                Ok(Line::End(f)) => footer = Some(f),
                Ok(Line::Header(_)) => return Err(TraceError::DuplicateHeader { line: lineno }),
                Err(e) => {
                    return Err(TraceError::Malformed {
                        line: lineno,
                        message: e.to_string(),
                    });
                }
            }
        }
        let footer = footer.ok_or(TraceError::Truncated {
            events: events.len() as u64,
        })?;
        if footer.events != events.len() as u64 {
            return Err(TraceError::CountMismatch {
                declared: footer.events,
                actual: events.len() as u64,
            });
        }
        if verify_checksum && footer.checksum != sum.hex() {
            return Err(TraceError::ChecksumMismatch {
                declared: footer.checksum,
                computed: sum.hex(),
            });
        }
        Ok(Trace {
            header,
            events,
            outcome: footer.outcome,
            message: footer.message,
        })
    }

    /// The decision values (random draws and choices) in order. This is the
    /// input the minimizer shrinks.
    pub fn decisions(&self) -> Vec<u64> {
        self.events
            .iter()
            .filter(|e| e.kind.is_decision())
            .filter_map(|e| match e.value {
                Value::U64(v) => Some(v),
                _ => None,
            })
            .collect()
    }

    /// Index of the first event where two traces differ, if any (comparing
    /// kind, key and value; call sites are ignored). Returns `Some(len)` when
    /// one trace is a strict prefix of the other.
    pub fn first_difference(&self, other: &Trace) -> Option<usize> {
        let n = self.events.len().min(other.events.len());
        for i in 0..n {
            let (a, b) = (&self.events[i], &other.events[i]);
            if a.kind != b.kind || a.key != b.key || a.value != b.value {
                return Some(i);
            }
        }
        (self.events.len() != other.events.len()).then_some(n)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::trace::{Blob, Event, Header, Kind, OutcomeKind};

    fn sample() -> Trace {
        Trace {
            header: Header::new("t", Some(7), "sim"),
            events: vec![
                Event {
                    seq: 0,
                    kind: Kind::ClockNow,
                    key: None,
                    value: Value::U64(5),
                    site: "a.rs:1:1".into(),
                },
                Event {
                    seq: 1,
                    kind: Kind::FsRead,
                    key: Some("x".into()),
                    value: Value::Bytes(Blob::from_bytes(&[0, 255])),
                    site: "a.rs:2:1".into(),
                },
            ],
            outcome: OutcomeKind::Failed,
            message: Some("boom".into()),
        }
    }

    #[test]
    fn round_trips() {
        let t = sample();
        assert_eq!(Trace::parse(&t.to_text()).unwrap(), t);
    }

    #[test]
    fn blob_hex_round_trips() {
        let bytes = [0u8, 1, 0x7f, 0x80, 0xff];
        assert_eq!(Blob::from_bytes(&bytes).to_bytes().unwrap(), bytes);
        assert!(Blob::Hex("abc".into()).to_bytes().is_err());
        assert!(Blob::Hex("zz".into()).to_bytes().is_err());
    }

    #[test]
    fn decisions_only_include_draws_and_choices() {
        let mut t = sample();
        t.events.push(Event {
            seq: 2,
            kind: Kind::RngU64,
            key: None,
            value: Value::U64(9),
            site: "s".into(),
        });
        t.events.push(Event {
            seq: 3,
            kind: Kind::Choose,
            key: Some("3".into()),
            value: Value::U64(2),
            site: "s".into(),
        });
        assert_eq!(t.decisions(), vec![9, 2]);
    }

    #[test]
    fn first_difference_ignores_sites() {
        let a = sample();
        let mut b = sample();
        b.events[0].site = "moved.rs:9:9".into();
        assert_eq!(a.first_difference(&b), None);
        b.events[1].key = Some("y".into());
        assert_eq!(a.first_difference(&b), Some(1));
        b.events.truncate(1);
        assert_eq!(a.first_difference(&b), Some(1));
    }
}

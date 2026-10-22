//! The on-disk trace format: a versioned, line-oriented JSON log.
//!
//! A trace file (`*.tmtrace`) has exactly three kinds of lines, in this order:
//!
//! ```text
//! {"header":{"format":"timemachine-trace","version":1,"test":"cache","seed":7,"sources":"sim","created_by":"…"}}
//! {"event":{"seq":0,"kind":"clock.now","value":{"u64":1735689600000000000},"site":"src/lib.rs:10:5"}}
//! …one line per recorded nondeterministic input, `seq` counting up from 0…
//! {"end":{"events":2,"outcome":"failed","message":"…","checksum":"9f3c…"}}
//! ```
//!
//! The footer's `events` count and FNV-1a checksum (over the exact bytes of the
//! event lines) let the reader distinguish a complete trace from a truncated or
//! hand-edited one. Every reader error names the line and says what to do.

use std::fmt;

use serde::{Deserialize, Serialize};

mod codec;
mod error;

pub use codec::fnv1a64;
pub use error::TraceError;

/// Value of the header's `format` field.
pub const FORMAT: &str = "timemachine-trace";

/// Trace format version written by this build. Readers reject other versions.
pub const VERSION: u32 = 1;

/// Kinds of recorded nondeterministic inputs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Kind {
    /// Reading the current time.
    #[serde(rename = "clock.now")]
    ClockNow,
    /// A sleep request (the key holds the requested nanoseconds).
    #[serde(rename = "clock.sleep")]
    ClockSleep,
    /// A 64-bit random draw.
    #[serde(rename = "rng.u64")]
    RngU64,
    /// A scheduling / ordering choice among `n` alternatives (key = `n`).
    #[serde(rename = "choose")]
    Choose,
    /// An environment-variable lookup (key = variable name).
    #[serde(rename = "env.var")]
    EnvVar,
    /// A file read (key = path).
    #[serde(rename = "fs.read")]
    FsRead,
    /// A file write (key = path, length and content hash).
    #[serde(rename = "fs.write")]
    FsWrite,
    /// An existence check (key = path).
    #[serde(rename = "fs.exists")]
    FsExists,
    /// A file removal (key = path).
    #[serde(rename = "fs.remove")]
    FsRemove,
    /// A directory listing (key = path).
    #[serde(rename = "fs.list")]
    FsList,
    /// A network request (key = method, target and body hash).
    #[serde(rename = "net.request")]
    NetRequest,
    /// A user checkpoint created with [`crate::Ctx::mark`] (key = label).
    #[serde(rename = "mark")]
    Mark,
}

impl Kind {
    /// The stable string used in trace files and diagnostics.
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::ClockNow => "clock.now",
            Kind::ClockSleep => "clock.sleep",
            Kind::RngU64 => "rng.u64",
            Kind::Choose => "choose",
            Kind::EnvVar => "env.var",
            Kind::FsRead => "fs.read",
            Kind::FsWrite => "fs.write",
            Kind::FsExists => "fs.exists",
            Kind::FsRemove => "fs.remove",
            Kind::FsList => "fs.list",
            Kind::NetRequest => "net.request",
            Kind::Mark => "mark",
        }
    }

    /// Whether this kind is a *decision* (a value the program draws rather than
    /// observes). Decisions are what the minimizer shrinks.
    pub fn is_decision(self) -> bool {
        matches!(self, Kind::RngU64 | Kind::Choose)
    }
}

impl fmt::Display for Kind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A byte payload, stored as UTF-8 text when possible and hex otherwise.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Blob {
    /// Valid UTF-8 content.
    Utf8(String),
    /// Arbitrary bytes, lowercase hex.
    Hex(String),
}

impl Blob {
    /// Encodes bytes.
    pub fn from_bytes(bytes: &[u8]) -> Blob {
        match std::str::from_utf8(bytes) {
            Ok(s) => Blob::Utf8(s.to_owned()),
            Err(_) => Blob::Hex(bytes.iter().map(|b| format!("{b:02x}")).collect()),
        }
    }

    /// Decodes bytes. Fails only on malformed hex.
    pub fn to_bytes(&self) -> Result<Vec<u8>, String> {
        match self {
            Blob::Utf8(s) => Ok(s.as_bytes().to_vec()),
            Blob::Hex(h) => {
                if h.len() % 2 != 0 {
                    return Err("hex blob has odd length".into());
                }
                (0..h.len())
                    .step_by(2)
                    .map(|i| {
                        h.get(i..i + 2)
                            .and_then(|pair| u8::from_str_radix(pair, 16).ok())
                            .ok_or_else(|| format!("invalid hex at offset {i}"))
                    })
                    .collect()
            }
        }
    }
}

/// A recorded error (I/O or network), reproduced on replay.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordedError {
    /// Error category, e.g. an `io::ErrorKind` name such as `NotFound`.
    pub kind: String,
    /// Human-readable message.
    pub message: String,
}

/// A recorded network response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordedResponse {
    /// Status code.
    pub status: u16,
    /// Response body.
    pub body: Blob,
}

/// The value observed for one event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Value {
    /// No payload (sleep, write success, marks).
    Unit,
    /// An integer (time in nanoseconds, random draw, choice index).
    U64(u64),
    /// A boolean (existence checks).
    Bool(bool),
    /// An optional string (environment variables).
    OptStr(Option<String>),
    /// Bytes (file contents).
    Bytes(Blob),
    /// A sorted list of names (directory listings).
    List(Vec<String>),
    /// A network response.
    Response(RecordedResponse),
    /// A failed operation.
    Err(RecordedError),
}

impl Value {
    /// Short description used in diagnostics (payloads are truncated).
    pub fn summary(&self) -> String {
        fn clip(s: &str) -> String {
            const MAX: usize = 48;
            if s.chars().count() <= MAX {
                format!("{s:?}")
            } else {
                let head: String = s.chars().take(MAX).collect();
                format!("{head:?}…")
            }
        }
        match self {
            Value::Unit => "()".into(),
            Value::U64(v) => v.to_string(),
            Value::Bool(b) => b.to_string(),
            Value::OptStr(None) => "unset".into(),
            Value::OptStr(Some(s)) => clip(s),
            Value::Bytes(Blob::Utf8(s)) => format!("{} bytes {}", s.len(), clip(s)),
            Value::Bytes(Blob::Hex(h)) => format!("{} bytes (binary)", h.len() / 2),
            Value::List(items) => format!("[{} entries]", items.len()),
            Value::Response(r) => format!("HTTP {}", r.status),
            Value::Err(e) => format!("error {}: {}", e.kind, clip(&e.message)),
        }
    }
}

/// One recorded nondeterministic input.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Event {
    /// Position in the trace, counting from 0.
    pub seq: u64,
    /// What kind of input this was.
    pub kind: Kind,
    /// The request that produced it (path, variable name, …), if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    /// The observed value.
    pub value: Value,
    /// Call site (`file:line:column`) that requested the input.
    pub site: String,
}

impl Event {
    /// One-line description used in diagnostics and `tm inspect`.
    pub fn describe(&self) -> String {
        let key = self
            .key
            .as_deref()
            .map(|k| format!(" {k:?}"))
            .unwrap_or_default();
        format!(
            "#{} {}{} = {}  at {}",
            self.seq,
            self.kind,
            key,
            self.value.summary(),
            self.site
        )
    }
}

/// The first line of a trace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Header {
    /// Always [`FORMAT`].
    pub format: String,
    /// Always [`VERSION`] for traces written by this build.
    pub version: u32,
    /// Name of the test or program that was recorded.
    pub test: String,
    /// Seed of the deterministic random source, when one was used.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed: Option<u64>,
    /// Which sources were active: `sim`, `real`, `drive` or `custom`.
    pub sources: String,
    /// Writer identification, e.g. `deterministic-timemachine 0.1.0`.
    pub created_by: String,
}

impl Header {
    /// A header for the current format version.
    pub fn new(test: impl Into<String>, seed: Option<u64>, sources: impl Into<String>) -> Header {
        Header {
            format: FORMAT.into(),
            version: VERSION,
            test: test.into(),
            seed,
            sources: sources.into(),
            created_by: concat!("deterministic-timemachine ", env!("CARGO_PKG_VERSION")).into(),
        }
    }
}

/// How the recorded run ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutcomeKind {
    /// The test returned success.
    Passed,
    /// The test returned an error.
    Failed,
    /// The test panicked.
    Panicked,
    /// Replay detected a divergence from the recorded trace.
    Diverged,
    /// The trace was saved without knowing the outcome.
    Unknown,
}

impl OutcomeKind {
    /// Whether this outcome counts as a test failure.
    pub fn is_failure(self) -> bool {
        matches!(self, OutcomeKind::Failed | OutcomeKind::Panicked)
    }
}

impl fmt::Display for OutcomeKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            OutcomeKind::Passed => "passed",
            OutcomeKind::Failed => "failed",
            OutcomeKind::Panicked => "panicked",
            OutcomeKind::Diverged => "diverged",
            OutcomeKind::Unknown => "unknown",
        })
    }
}

/// The last line of a trace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Footer {
    /// Number of event lines.
    pub events: u64,
    /// How the recorded run ended.
    pub outcome: OutcomeKind,
    /// Failure message, when there was one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// FNV-1a 64 checksum of the event lines, lowercase hex.
    pub checksum: String,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Line {
    Header(Header),
    Event(Event),
    End(Footer),
}

/// A complete, validated trace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Trace {
    /// The header line.
    pub header: Header,
    /// Recorded events in order.
    pub events: Vec<Event>,
    /// How the run ended.
    pub outcome: OutcomeKind,
    /// Failure message, if any.
    pub message: Option<String>,
}

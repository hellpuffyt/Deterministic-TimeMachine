//! The public input methods of [`Ctx`]: time, randomness, environment,
//! files, network and checkpoints. Each one funnels through the engine's
//! `interact`, which records, replays or drives the value.

use std::io;
use std::panic::Location;
use std::path::Path;
use std::time::Duration;

use super::Ctx;
use crate::sources::clock::saturating_nanos;
use crate::sources::fs::confine;
use crate::sources::{NetError, Request, Response};
use crate::trace::{Blob, Kind, RecordedError, RecordedResponse, Value, fnv1a64};

fn path_key(path: &Path) -> String {
    match confine(path) {
        Ok(p) => p
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join("/"),
        Err(_) => path.display().to_string(),
    }
}

fn io_err_value(e: &io::Error) -> Value {
    Value::Err(RecordedError {
        kind: format!("{:?}", e.kind()),
        message: e.to_string(),
    })
}

fn io_err_from(rec: &RecordedError) -> io::Error {
    use io::ErrorKind as K;
    let kind = match rec.kind.as_str() {
        "NotFound" => K::NotFound,
        "PermissionDenied" => K::PermissionDenied,
        "AlreadyExists" => K::AlreadyExists,
        "InvalidInput" => K::InvalidInput,
        "InvalidData" => K::InvalidData,
        "TimedOut" => K::TimedOut,
        "Interrupted" => K::Interrupted,
        "UnexpectedEof" => K::UnexpectedEof,
        "WouldBlock" => K::WouldBlock,
        "ConnectionRefused" => K::ConnectionRefused,
        "ConnectionReset" => K::ConnectionReset,
        "BrokenPipe" => K::BrokenPipe,
        _ => K::Other,
    };
    io::Error::new(kind, rec.message.clone())
}

impl Ctx {
    // ----- time ---------------------------------------------------------

    /// Current time in nanoseconds since the Unix epoch.
    #[track_caller]
    pub fn now_nanos(&self) -> u64 {
        let loc = Location::caller();
        let v = self.interact(Kind::ClockNow, None, loc, None, |s| {
            Value::U64(s.clock.now_nanos())
        });
        self.u64_value(Kind::ClockNow, None, loc, v)
    }

    /// Current time as a duration since the Unix epoch.
    #[track_caller]
    pub fn now(&self) -> Duration {
        Duration::from_nanos(self.now_nanos())
    }

    /// Sleeps (really, or by advancing virtual time; never during replay).
    #[track_caller]
    pub fn sleep(&self, duration: Duration) {
        let nanos = saturating_nanos(duration);
        self.interact(
            Kind::ClockSleep,
            Some(nanos.to_string()),
            Location::caller(),
            None,
            |s| {
                s.clock.sleep(duration);
                Value::Unit
            },
        );
    }

    // ----- randomness ---------------------------------------------------

    /// A random 64-bit value.
    #[track_caller]
    pub fn next_u64(&self) -> u64 {
        let loc = Location::caller();
        let v = self.interact(Kind::RngU64, None, loc, None, |s| {
            Value::U64(s.rng.next_u64())
        });
        self.u64_value(Kind::RngU64, None, loc, v)
    }

    /// A random value in `range` (uniform up to a negligible modulo bias).
    ///
    /// # Panics
    /// If the range is empty.
    #[track_caller]
    pub fn gen_range(&self, range: std::ops::Range<u64>) -> u64 {
        assert!(
            range.start < range.end,
            "gen_range called with an empty range {range:?}"
        );
        range.start + self.next_u64() % (range.end - range.start)
    }

    /// `true` with probability `p` (clamped to `[0, 1]`).
    #[track_caller]
    pub fn gen_bool(&self, p: f64) -> bool {
        // 53 random bits → uniform f64 in [0, 1).
        let unit = (self.next_u64() >> 11) as f64 * (1.0 / (1u64 << 53) as f64);
        unit < p.clamp(0.0, 1.0)
    }

    /// Chooses one of `n` alternatives. This is the hook for controlling
    /// orderings and interleavings: every choice is recorded and replayed.
    ///
    /// # Panics
    /// If `n == 0`.
    #[track_caller]
    pub fn choose(&self, n: usize) -> usize {
        assert!(n > 0, "choose(0): there must be at least one alternative");
        let loc = Location::caller();
        let n64 = n as u64;
        let key = Some(n.to_string());
        let v = self.interact(Kind::Choose, key.clone(), loc, Some(n64), |s| {
            Value::U64(s.rng.next_u64() % n64)
        });
        let idx = self.u64_value(Kind::Choose, key.clone(), loc, v);
        if idx >= n64 {
            self.bad_value(Kind::Choose, key, loc, "in-range choice", &Value::U64(idx));
        }
        idx as usize
    }

    /// Shuffles a slice (Fisher–Yates over recorded choices).
    #[track_caller]
    pub fn shuffle<T>(&self, items: &mut [T]) {
        for i in (1..items.len()).rev() {
            let j = self.choose(i + 1);
            items.swap(i, j);
        }
    }

    // ----- environment --------------------------------------------------

    /// An environment variable.
    #[track_caller]
    pub fn env_var(&self, name: &str) -> Option<String> {
        let loc = Location::caller();
        let key = Some(name.to_owned());
        match self.interact(Kind::EnvVar, key.clone(), loc, None, |s| {
            Value::OptStr(s.env.var(name))
        }) {
            Value::OptStr(v) => v,
            other => self.bad_value(Kind::EnvVar, key, loc, "optional string", &other),
        }
    }

    // ----- filesystem ---------------------------------------------------

    /// Reads a whole file.
    #[track_caller]
    pub fn read(&self, path: impl AsRef<Path>) -> io::Result<Vec<u8>> {
        let path = path.as_ref();
        let loc = Location::caller();
        let key = Some(path_key(path));
        let v = self.interact(Kind::FsRead, key.clone(), loc, None, |s| {
            match s.fs.read(path) {
                Ok(b) => Value::Bytes(Blob::from_bytes(&b)),
                Err(e) => io_err_value(&e),
            }
        });
        match v {
            Value::Bytes(b) => match b.to_bytes() {
                Ok(bytes) => Ok(bytes),
                Err(msg) => self.bad_value(
                    Kind::FsRead,
                    key,
                    loc,
                    "bytes",
                    &Value::Err(RecordedError {
                        kind: "InvalidData".into(),
                        message: msg,
                    }),
                ),
            },
            Value::Err(e) => Err(io_err_from(&e)),
            other => self.bad_value(Kind::FsRead, key, loc, "bytes", &other),
        }
    }

    /// Reads a whole file as UTF-8.
    #[track_caller]
    pub fn read_to_string(&self, path: impl AsRef<Path>) -> io::Result<String> {
        let bytes = self.read(path)?;
        String::from_utf8(bytes).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
    }

    /// Writes a file. The trace records the path, length and a content hash,
    /// so replay detects a program that writes different data.
    #[track_caller]
    pub fn write(&self, path: impl AsRef<Path>, contents: impl AsRef<[u8]>) -> io::Result<()> {
        let (path, contents) = (path.as_ref(), contents.as_ref());
        let loc = Location::caller();
        let key = Some(format!(
            "{} len={} fnv={:016x}",
            path_key(path),
            contents.len(),
            fnv1a64(contents)
        ));
        match self.interact(Kind::FsWrite, key.clone(), loc, None, |s| {
            match s.fs.write(path, contents) {
                Ok(()) => Value::Unit,
                Err(e) => io_err_value(&e),
            }
        }) {
            Value::Unit => Ok(()),
            Value::Err(e) => Err(io_err_from(&e)),
            other => self.bad_value(Kind::FsWrite, key, loc, "unit", &other),
        }
    }

    /// Whether a path exists.
    #[track_caller]
    pub fn exists(&self, path: impl AsRef<Path>) -> bool {
        let path = path.as_ref();
        let loc = Location::caller();
        let key = Some(path_key(path));
        match self.interact(Kind::FsExists, key.clone(), loc, None, |s| {
            Value::Bool(s.fs.exists(path))
        }) {
            Value::Bool(b) => b,
            other => self.bad_value(Kind::FsExists, key, loc, "bool", &other),
        }
    }

    /// Removes a file.
    #[track_caller]
    pub fn remove(&self, path: impl AsRef<Path>) -> io::Result<()> {
        let path = path.as_ref();
        let loc = Location::caller();
        let key = Some(path_key(path));
        match self.interact(Kind::FsRemove, key.clone(), loc, None, |s| {
            match s.fs.remove(path) {
                Ok(()) => Value::Unit,
                Err(e) => io_err_value(&e),
            }
        }) {
            Value::Unit => Ok(()),
            Value::Err(e) => Err(io_err_from(&e)),
            other => self.bad_value(Kind::FsRemove, key, loc, "unit", &other),
        }
    }

    /// Lists a directory (sorted names).
    #[track_caller]
    pub fn list(&self, path: impl AsRef<Path>) -> io::Result<Vec<String>> {
        let path = path.as_ref();
        let loc = Location::caller();
        let key = Some(path_key(path));
        match self.interact(Kind::FsList, key.clone(), loc, None, |s| {
            match s.fs.list(path) {
                Ok(names) => Value::List(names),
                Err(e) => io_err_value(&e),
            }
        }) {
            Value::List(names) => Ok(names),
            Value::Err(e) => Err(io_err_from(&e)),
            other => self.bad_value(Kind::FsList, key, loc, "list", &other),
        }
    }

    // ----- network ------------------------------------------------------

    /// Performs a network exchange through the configured transport.
    #[track_caller]
    pub fn request(&self, req: &Request) -> Result<Response, NetError> {
        let loc = Location::caller();
        let key = Some(format!(
            "{} {} len={} fnv={:016x}",
            req.method,
            req.target,
            req.body.len(),
            fnv1a64(&req.body)
        ));
        match self.interact(Kind::NetRequest, key.clone(), loc, None, |s| {
            match s.net.request(req) {
                Ok(r) => Value::Response(RecordedResponse {
                    status: r.status,
                    body: Blob::from_bytes(&r.body),
                }),
                Err(e) => Value::Err(RecordedError {
                    kind: e.kind,
                    message: e.message,
                }),
            }
        }) {
            Value::Response(r) => match r.body.to_bytes() {
                Ok(body) => Ok(Response {
                    status: r.status,
                    body,
                }),
                Err(msg) => Err(NetError::new("corrupt_trace", msg)),
            },
            Value::Err(e) => Err(NetError::new(e.kind, e.message)),
            other => self.bad_value(Kind::NetRequest, key, loc, "response", &other),
        }
    }

    // ----- checkpoints and introspection --------------------------------

    /// Records a named checkpoint. Replay verifies checkpoints are reached in
    /// the same order, and divergence reports show them as context.
    #[track_caller]
    pub fn mark(&self, label: &str) {
        self.interact(
            Kind::Mark,
            Some(label.to_owned()),
            Location::caller(),
            None,
            |_| Value::Unit,
        );
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn path_keys_are_normalized() {
        assert_eq!(path_key(Path::new("./a//b")), "a/b");
        assert_eq!(path_key(Path::new("../x")), "../x");
    }

    #[test]
    fn io_errors_round_trip_kind() {
        let e = io::Error::new(io::ErrorKind::NotFound, "gone");
        let Value::Err(rec) = io_err_value(&e) else {
            panic!("expected error value")
        };
        assert_eq!(io_err_from(&rec).kind(), io::ErrorKind::NotFound);
    }
}

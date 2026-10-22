//! Errors from reading trace files.

use std::io;

use super::FORMAT;

/// Reasons a trace could not be read. Every variant says how to proceed.
#[derive(Debug, thiserror::Error)]
pub enum TraceError {
    /// The file could not be read or written.
    #[error("cannot access trace file: {0}")]
    Io(#[from] io::Error),
    /// The file is empty.
    #[error(
        "trace is empty; expected a header line (was the recording interrupted before it started?)"
    )]
    Empty,
    /// The first line is not a header.
    #[error("line 1 is not a trace header; this file was not written by timemachine")]
    MissingHeader,
    /// The header names a different format.
    #[error("unknown trace format {found:?}; expected {FORMAT:?}")]
    WrongFormat {
        /// What the file declared.
        found: String,
    },
    /// The trace was written by an incompatible version.
    #[error(
        "trace format version {found} is not supported (this build reads version {supported}); re-record the trace with this version of timemachine"
    )]
    UnsupportedVersion {
        /// Version in the file.
        found: u32,
        /// Version this build supports.
        supported: u32,
    },
    /// A line is not valid JSON of the expected shape.
    #[error("line {line}: malformed trace line ({message})")]
    Malformed {
        /// 1-based line number.
        line: usize,
        /// Parser message.
        message: String,
    },
    /// Event sequence numbers are not contiguous.
    #[error(
        "line {line}: expected event seq {expected}, found {found}; events were reordered or removed"
    )]
    BadSequence {
        /// 1-based line number.
        line: usize,
        /// Expected seq.
        expected: u64,
        /// Seq found.
        found: u64,
    },
    /// A header appeared after line 1.
    #[error("line {line}: unexpected second header")]
    DuplicateHeader {
        /// 1-based line number.
        line: usize,
    },
    /// The footer is missing.
    #[error(
        "trace is truncated: {events} events and no end line (the recorded process probably crashed or was killed before finishing)"
    )]
    Truncated {
        /// Events that were readable.
        events: u64,
    },
    /// Content follows the footer.
    #[error("line {line}: content after the end line")]
    TrailingData {
        /// 1-based line number.
        line: usize,
    },
    /// The footer's event count disagrees with the file.
    #[error("end line says {declared} events but the file has {actual}")]
    CountMismatch {
        /// Count in the footer.
        declared: u64,
        /// Count found.
        actual: u64,
    },
    /// The checksum does not match the event lines.
    #[error(
        "checksum mismatch (expected {declared}, computed {computed}): the event lines were modified or corrupted; read with verification disabled only if the edit was intentional"
    )]
    ChecksumMismatch {
        /// Checksum in the footer.
        declared: String,
        /// Checksum computed from the file.
        computed: String,
    },
}

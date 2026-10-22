//! # timemachine — make flaky tests reproducible
//!
//! Route every nondeterministic input your code reads — time, randomness,
//! files, environment variables, network responses, and scheduling choices —
//! through a [`Ctx`]. Then:
//!
//! * **record** a run to a versioned, line-oriented trace,
//! * **replay** it: the same recorded inputs produce the same observable
//!   behavior, and any deviation is reported as a precise [`Divergence`]
//!   (expected vs actual input, event index, call site, preceding events),
//! * **explore** many seeds with simulated sources to find a failing one,
//! * **minimize** a failing trace with delta debugging.
//!
//! This is explicit dependency injection, not syscall interception: only
//! inputs that go through a `Ctx` are controlled. See `docs/UNSUPPORTED.md`.
//!
//! ```
//! use std::time::Duration;
//! use timemachine::{Builder, harness};
//!
//! // Code under test reads time and randomness only through `ctx`.
//! fn backoff_fits_deadline(ctx: &timemachine::Ctx) -> Result<(), String> {
//!     let start = ctx.now();
//!     ctx.sleep(Duration::from_millis(90 + ctx.gen_range(0..20)));
//!     let waited = ctx.now() - start;
//!     if waited > Duration::from_millis(100) {
//!         return Err(format!("waited {waited:?}, deadline is 100ms"));
//!     }
//!     Ok(())
//! }
//!
//! // Find a failing seed (virtual time: this takes microseconds).
//! let found = harness::explore("backoff", Default::default(), |b| b, backoff_fits_deadline)
//!     .first_failure
//!     .expect("about half of all seeds fail");
//!
//! // The captured trace reproduces the failure exactly, every time.
//! let replay = harness::replay(&found.trace, |b| b, backoff_fits_deadline);
//! assert!(replay.reproduced);
//! ```

#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]

pub mod ctx;
pub mod divergence;
pub mod envmode;
pub mod harness;
pub mod minimize;
pub mod sched;
pub mod sources;
pub mod trace;

pub use ctx::{Builder, Ctx, Mode, SiteCheck};
pub use divergence::{Divergence, DivergenceReason};
pub use envmode::{RunReport, run_main};
pub use trace::{Event, Kind, OutcomeKind, Trace, TraceError};

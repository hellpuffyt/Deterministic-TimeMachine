//! Pluggable sources of nondeterminism.
//!
//! Each trait is tiny on purpose. A [`crate::Ctx`] owns one of each; in
//! record mode it calls them and writes what they returned to the trace, in
//! replay mode it never calls them and answers from the trace instead.

pub mod clock;
pub mod env;
pub mod fs;
pub mod net;
pub mod rng;

pub use clock::{Clock, SystemClock, VirtualClock};
pub use env::{Env, MapEnv, ProcessEnv};
pub use fs::{Fs, MemFs, RealFs};
pub use net::{Net, NetError, NoNet, Request, Response, StubNet};
pub use rng::{EntropyRng, RngSource, SeededRng};

/// One of each source.
pub struct Sources {
    /// Time.
    pub clock: Box<dyn Clock>,
    /// Randomness.
    pub rng: Box<dyn RngSource>,
    /// Files.
    pub fs: Box<dyn Fs>,
    /// Environment variables.
    pub env: Box<dyn Env>,
    /// Network.
    pub net: Box<dyn Net>,
}

impl Sources {
    /// Fully simulated sources: virtual clock, seeded RNG, empty in-memory
    /// filesystem, empty environment, no network. Deterministic for a given
    /// seed.
    pub fn simulated(seed: u64) -> Sources {
        Sources {
            clock: Box::new(VirtualClock::new()),
            rng: Box::new(SeededRng::new(seed)),
            fs: Box::new(MemFs::new()),
            env: Box::new(MapEnv::new()),
            net: Box::new(NoNet),
        }
    }

    /// Real sources: system clock, entropy-seeded RNG, the real filesystem
    /// confined to the current directory, the process environment and no
    /// network (configure one explicitly).
    pub fn real() -> Sources {
        Sources {
            clock: Box::new(SystemClock),
            rng: Box::new(EntropyRng::new()),
            fs: Box::new(RealFs::new(".")),
            env: Box::new(ProcessEnv),
            net: Box::new(NoNet),
        }
    }
}

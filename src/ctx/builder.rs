//! Configuring a [`Ctx`]: sources, seed, site checking, and mode.

use std::sync::{Arc, Mutex};

use super::engine::{Engine, State};
use super::{Ctx, Inner, Mode, Persist, SiteCheck};
use crate::sources::{Clock, Env, Fs, Net, RngSource, Sources};
use crate::trace::Trace;

/// Configures and creates a [`Ctx`].
pub struct Builder {
    test: String,
    seed: Option<u64>,
    real: bool,
    clock: Option<Box<dyn Clock>>,
    rng: Option<Box<dyn RngSource>>,
    fs: Option<Box<dyn Fs>>,
    env: Option<Box<dyn Env>>,
    net: Option<Box<dyn Net>>,
    site_check: SiteCheck,
    pub(crate) persist: Persist,
}

impl Builder {
    /// A builder for the test or program called `test` (stored in traces).
    /// By default every source is simulated (see [`Sources::simulated`]).
    pub fn new(test: impl Into<String>) -> Builder {
        Builder {
            test: test.into(),
            seed: None,
            real: false,
            clock: None,
            rng: None,
            fs: None,
            env: None,
            net: None,
            site_check: SiteCheck::default(),
            persist: Persist::default(),
        }
    }

    /// Seed for the simulated random source.
    pub fn seed(mut self, seed: u64) -> Self {
        self.seed = Some(seed);
        self
    }

    /// Use real sources (system clock, entropy, real files under `.`,
    /// process environment) for anything not set explicitly.
    pub fn real_sources(mut self) -> Self {
        self.real = true;
        self
    }

    /// Whether real sources were requested.
    pub fn uses_real_sources(&self) -> bool {
        self.real
    }

    /// The test name.
    pub fn test_name(&self) -> &str {
        &self.test
    }

    /// Sets the clock.
    pub fn clock(mut self, clock: impl Clock + 'static) -> Self {
        self.clock = Some(Box::new(clock));
        self
    }

    /// Sets the random source.
    pub fn rng(mut self, rng: impl RngSource + 'static) -> Self {
        self.rng = Some(Box::new(rng));
        self
    }

    /// Sets the filesystem.
    pub fn fs(mut self, fs: impl Fs + 'static) -> Self {
        self.fs = Some(Box::new(fs));
        self
    }

    /// Sets the environment.
    pub fn env(mut self, env: impl Env + 'static) -> Self {
        self.env = Some(Box::new(env));
        self
    }

    /// Sets the network transport.
    pub fn net(mut self, net: impl Net + 'static) -> Self {
        self.net = Some(Box::new(net));
        self
    }

    /// Sets how replay compares call sites.
    pub fn site_check(mut self, check: SiteCheck) -> Self {
        self.site_check = check;
        self
    }

    pub(crate) fn real_flag(&mut self, real: bool) {
        self.real = real;
    }

    fn into_parts(self, label_override: Option<&str>) -> (Sources, PartialInner) {
        let custom = self.clock.is_some()
            || self.rng.is_some()
            || self.fs.is_some()
            || self.env.is_some()
            || self.net.is_some();
        let defaults = if self.real {
            Sources::real()
        } else {
            Sources::simulated(self.seed.unwrap_or(0))
        };
        let sources = Sources {
            clock: self.clock.unwrap_or(defaults.clock),
            rng: self.rng.unwrap_or(defaults.rng),
            fs: self.fs.unwrap_or(defaults.fs),
            env: self.env.unwrap_or(defaults.env),
            net: self.net.unwrap_or(defaults.net),
        };
        let base = if self.real { "real" } else { "sim" };
        let label = match label_override {
            Some(l) => l.to_owned(),
            None if custom => format!("{base}+custom"),
            None => base.to_owned(),
        };
        (
            sources,
            PartialInner {
                test: self.test,
                seed: if self.real || label_override == Some("drive") {
                    None
                } else {
                    Some(self.seed.unwrap_or(0))
                },
                label,
                site_check: self.site_check,
                persist: self.persist,
            },
        )
    }

    fn build(self, mode: Mode, engine: Engine, label: Option<&str>) -> Ctx {
        let (sources, p) = self.into_parts(label);
        Ctx {
            inner: Arc::new(Inner {
                state: Mutex::new(State {
                    sources,
                    engine,
                    divergence: None,
                    finished: false,
                }),
                test: p.test,
                seed: p.seed,
                sources_label: p.label,
                site_check: p.site_check,
                mode,
                persist: p.persist,
            }),
        }
    }

    /// A context that uses the sources directly and records nothing.
    pub fn live(self) -> Ctx {
        self.build(Mode::Live, Engine::Live, None)
    }

    /// A context that records every input.
    pub fn record(self) -> Ctx {
        self.build(Mode::Record, Engine::Record(Vec::new()), None)
    }

    /// A context that answers every request from `trace`.
    pub fn replay(self, trace: Trace) -> Ctx {
        let seed = trace.header.seed;
        let label = trace.header.sources.clone();
        let mut b = self;
        b.seed = seed;
        b.build(
            Mode::Replay,
            Engine::Replay { trace, cursor: 0 },
            Some(&label),
        )
    }

    /// A context that takes decisions from `decisions` (then zeros once
    /// exhausted) and records the run.
    pub fn drive(self, decisions: Vec<u64>) -> Ctx {
        self.build(
            Mode::Drive,
            Engine::Drive {
                decisions: decisions.into(),
                events: Vec::new(),
            },
            Some("drive"),
        )
    }
}

struct PartialInner {
    test: String,
    seed: Option<u64>,
    label: String,
    site_check: SiteCheck,
    persist: Persist,
}

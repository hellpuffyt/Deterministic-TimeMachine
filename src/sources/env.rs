//! Environment-variable sources.

use std::collections::BTreeMap;

/// A source of environment variables.
pub trait Env: Send {
    /// The value of `name`, or `None` if unset (or not valid UTF-8).
    fn var(&mut self, name: &str) -> Option<String>;
}

/// The real process environment. Non-UTF-8 values read as unset.
#[derive(Debug, Clone, Default)]
pub struct ProcessEnv;

impl Env for ProcessEnv {
    fn var(&mut self, name: &str) -> Option<String> {
        std::env::var(name).ok()
    }
}

/// A fixed, in-memory environment.
#[derive(Debug, Clone, Default)]
pub struct MapEnv {
    vars: BTreeMap<String, String>,
}

impl MapEnv {
    /// An empty environment.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a variable (builder style).
    pub fn with(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.vars.insert(name.into(), value.into());
        self
    }
}

impl Env for MapEnv {
    fn var(&mut self, name: &str) -> Option<String> {
        self.vars.get(name).cloned()
    }
}

//! Time sources.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// A source of time. Implementations must be `Send` so a [`crate::Ctx`] can
/// move between threads.
pub trait Clock: Send {
    /// Current time in nanoseconds since the Unix epoch.
    fn now_nanos(&mut self) -> u64;
    /// Waits for `duration` (really, or by advancing virtual time).
    fn sleep(&mut self, duration: Duration);
}

/// Virtual time: `now` only moves when the program sleeps or the clock is
/// advanced. Sleeping returns immediately.
#[derive(Debug, Clone)]
pub struct VirtualClock {
    now: u64,
}

/// 2025-01-01T00:00:00Z, the default starting point of [`VirtualClock`], so
/// programs that format wall-clock time see a plausible date.
pub const DEFAULT_EPOCH_NANOS: u64 = 1_735_689_600_000_000_000;

impl VirtualClock {
    /// A clock starting at [`DEFAULT_EPOCH_NANOS`].
    pub fn new() -> Self {
        Self::starting_at(DEFAULT_EPOCH_NANOS)
    }

    /// A clock starting at `nanos` since the Unix epoch.
    pub fn starting_at(nanos: u64) -> Self {
        VirtualClock { now: nanos }
    }

    /// Moves time forward without a sleep.
    pub fn advance(&mut self, by: Duration) {
        self.now = self.now.saturating_add(saturating_nanos(by));
    }
}

impl Default for VirtualClock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock for VirtualClock {
    fn now_nanos(&mut self) -> u64 {
        self.now
    }

    fn sleep(&mut self, duration: Duration) {
        self.advance(duration);
    }
}

/// The operating system's wall clock; `sleep` really blocks the thread.
#[derive(Debug, Clone, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_nanos(&mut self) -> u64 {
        // A clock before 1970 is treated as the epoch rather than failing.
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(saturating_nanos)
            .unwrap_or(0)
    }

    fn sleep(&mut self, duration: Duration) {
        std::thread::sleep(duration);
    }
}

/// Duration as nanoseconds, saturating at `u64::MAX` (~584 years).
pub fn saturating_nanos(d: Duration) -> u64 {
    u64::try_from(d.as_nanos()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn virtual_clock_only_moves_on_sleep() {
        let mut c = VirtualClock::starting_at(10);
        assert_eq!(c.now_nanos(), 10);
        assert_eq!(c.now_nanos(), 10);
        c.sleep(Duration::from_nanos(5));
        assert_eq!(c.now_nanos(), 15);
        c.advance(Duration::MAX);
        assert_eq!(c.now_nanos(), u64::MAX);
    }
}

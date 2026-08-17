//! Monotonic time source for host loops and time-sensitive input gestures.

use std::time::Instant;

/// A monotonic clock that can be replaced in deterministic hosts and tests.
///
/// Returned instants must share one monotonic epoch and must not move backward.
/// Components use elapsed durations only; wall-clock dates and time zones are
/// intentionally outside this boundary.
pub trait Clock {
    /// Read the current monotonic instant.
    fn now(&self) -> Instant;
}

/// The process monotonic clock backed by [`Instant::now`].
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
}

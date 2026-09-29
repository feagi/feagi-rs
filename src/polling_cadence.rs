//! Cadence for the agent handler polling loop.
//!
//! The loop moves sensory packets from the transports into the burst engine's
//! intake queue and stamps their arrival time. It used to end every cycle with a
//! fixed 10 ms sleep, so one cycle took 10 ms plus its own work: about 85 cycles
//! per second on macOS. An agent sending once per 10 ms burst then had two
//! packets waiting every sixth or seventh cycle. Both were stamped together,
//! landed in the same burst, and the burst engine kept only the newest.
//!
//! Deadlines here sit on a fixed grid, so the time a cycle spends working does
//! not lengthen the period.
//!
//! @cursor:critical-path Runs once per polling cycle.

use std::time::{Duration, Instant};

/// Deadlines on a fixed grid of `interval`.
#[derive(Debug, Clone)]
pub struct PollingCadence {
    interval: Duration,
    next: Instant,
}

impl PollingCadence {
    /// Start a grid whose first deadline is one interval after `now`.
    ///
    /// # Errors
    ///
    /// Returns an error when `interval_ms` is not a positive, finite number.
    pub fn new(interval_ms: f64, now: Instant) -> Result<Self, String> {
        if !interval_ms.is_finite() || interval_ms <= 0.0 {
            return Err(format!(
                "agent.polling_interval_ms must be a positive number of milliseconds (got {interval_ms})"
            ));
        }
        let interval = Duration::from_secs_f64(interval_ms / 1000.0);
        Ok(Self {
            interval,
            next: now + interval,
        })
    }

    pub fn interval(&self) -> Duration {
        self.interval
    }

    /// Deadline the current cycle should sleep until.
    pub fn deadline(&self) -> Instant {
        self.next
    }

    /// How long to sleep from `now` until the current deadline. Zero when it has passed.
    pub fn sleep_for(&self, now: Instant) -> Duration {
        self.next.saturating_duration_since(now)
    }

    /// Move to the next deadline once the cycle that ended at `now` has slept.
    ///
    /// A cycle that ran past the next deadline starts a new grid from `now`
    /// rather than running the missed cycles back to back.
    pub fn advance(&mut self, now: Instant) {
        let next = self.next + self.interval;
        self.next = if next <= now {
            now + self.interval
        } else {
            next
        };
    }
}

#[cfg(test)]
mod tests {
    use super::PollingCadence;
    use std::time::{Duration, Instant};

    #[test]
    fn rejects_non_positive_or_non_finite_intervals() {
        let now = Instant::now();
        for bad in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert!(
                PollingCadence::new(bad, now).is_err(),
                "{bad} should be rejected"
            );
        }
    }

    #[test]
    fn work_inside_a_cycle_does_not_lengthen_the_period() {
        let start = Instant::now();
        let mut cadence = PollingCadence::new(1.0, start).expect("valid interval");
        let interval = cadence.interval();
        // Every cycle works 0.3 ms and wakes 0.2 ms late.
        for n in 1..=1000u32 {
            let deadline = cadence.deadline();
            assert_eq!(deadline, start + interval * n);
            let woke = deadline + Duration::from_micros(200);
            cadence.advance(woke);
        }
        assert_eq!(cadence.deadline(), start + interval * 1001);
    }

    #[test]
    fn a_stalled_cycle_starts_a_new_grid_instead_of_bursting() {
        let start = Instant::now();
        let mut cadence = PollingCadence::new(1.0, start).expect("valid interval");
        let stalled = start + Duration::from_millis(25);
        cadence.advance(stalled);
        assert_eq!(cadence.deadline(), stalled + cadence.interval());
    }

    #[test]
    fn sleep_is_zero_once_the_deadline_has_passed() {
        let start = Instant::now();
        let cadence = PollingCadence::new(2.0, start).expect("valid interval");
        assert_eq!(cadence.sleep_for(start), Duration::from_millis(2));
        assert_eq!(
            cadence.sleep_for(start + Duration::from_millis(5)),
            Duration::ZERO
        );
    }
}

//! Hybrid logical clock.
//!
//! The mesh orders events without a coordinator. Wall clocks on two of a
//! user's machines disagree by seconds; a pure Lamport counter loses all
//! relation to real time and makes a timeline unreadable. An HLC keeps both:
//! it never runs backwards, it stays within clock skew of real time, and it
//! preserves causality across nodes.
//!
//! Time is a parameter here for the same reason it is in `heiwa_identity`:
//! a clock this crate reads itself cannot be tested.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct HybridLogicalClock {
    /// Milliseconds since the Unix epoch, monotonised.
    pub wall_ms: u64,
    /// Disambiguates events sharing a `wall_ms`.
    pub counter: u32,
}

impl HybridLogicalClock {
    /// Stamp a locally generated event.
    pub fn tick(&self, now_ms: u64) -> Self {
        if now_ms > self.wall_ms {
            Self {
                wall_ms: now_ms,
                counter: 0,
            }
        } else {
            Self::step(self.wall_ms, self.counter)
        }
    }

    /// Stamp the receipt of a peer event, absorbing its clock.
    pub fn observe(&self, remote: &Self, now_ms: u64) -> Self {
        let highest = self.wall_ms.max(remote.wall_ms);
        if now_ms > highest {
            return Self {
                wall_ms: now_ms,
                counter: 0,
            };
        }
        let counter = if self.wall_ms == remote.wall_ms {
            self.counter.max(remote.counter)
        } else if self.wall_ms > remote.wall_ms {
            self.counter
        } else {
            remote.counter
        };
        Self::step(highest, counter)
    }

    /// One logical step past `(wall_ms, counter)`, without overflowing.
    ///
    /// The counter is finite, so exhaustion needs a policy rather than a `+ 1`
    /// that panics in debug builds and wraps to zero in release ones — wrapping
    /// would sort the new stamp *before* the event it follows, which is the one
    /// thing an HLC exists to prevent. The carry goes into the wall clock: a
    /// millisecond later with a fresh counter is strictly greater, and it stays
    /// within skew because a counter only reaches `u32::MAX` when four billion
    /// events already shared a single millisecond.
    fn step(wall_ms: u64, counter: u32) -> Self {
        match counter.checked_add(1) {
            Some(counter) => Self { wall_ms, counter },
            None => match wall_ms.checked_add(1) {
                Some(wall_ms) => Self {
                    wall_ms,
                    counter: 0,
                },
                // Both components saturated (~585 million years past the
                // epoch). Hold the stamp rather than regress: no later value
                // is representable, and standing still is survivable where
                // going backwards is not.
                None => Self { wall_ms, counter },
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tick_follows_the_wall_clock_when_it_has_advanced() {
        let clock = HybridLogicalClock {
            wall_ms: 100,
            counter: 4,
        };
        let next = clock.tick(200);
        assert_eq!(next.wall_ms, 200);
        assert_eq!(next.counter, 0, "a new millisecond restarts the counter");
    }

    #[test]
    fn a_tick_within_the_same_millisecond_advances_the_counter() {
        let clock = HybridLogicalClock {
            wall_ms: 100,
            counter: 4,
        };
        let next = clock.tick(100);
        assert_eq!(next.wall_ms, 100);
        assert_eq!(next.counter, 5);
    }

    #[test]
    fn a_backwards_wall_clock_never_moves_the_hlc_backwards() {
        let clock = HybridLogicalClock {
            wall_ms: 100,
            counter: 0,
        };
        let next = clock.tick(40);
        assert_eq!(
            next.wall_ms, 100,
            "an NTP correction must not rewrite history"
        );
        assert_eq!(next.counter, 1);
        assert!(next > clock, "every stamp is strictly later than the last");
    }

    #[test]
    fn observing_a_peer_ahead_of_us_absorbs_its_clock() {
        let local = HybridLogicalClock {
            wall_ms: 100,
            counter: 0,
        };
        let remote = HybridLogicalClock {
            wall_ms: 500,
            counter: 3,
        };
        let next = local.observe(&remote, 120);
        assert_eq!(next.wall_ms, 500, "a skewed peer still orders correctly");
        assert_eq!(next.counter, 4);
        assert!(next > remote, "receipt happens after send");
    }

    #[test]
    fn observing_a_peer_behind_us_keeps_our_own_clock() {
        let local = HybridLogicalClock {
            wall_ms: 900,
            counter: 2,
        };
        let remote = HybridLogicalClock {
            wall_ms: 100,
            counter: 0,
        };
        let next = local.observe(&remote, 900);
        assert_eq!(next.wall_ms, 900);
        assert_eq!(next.counter, 3);
    }

    #[test]
    fn a_tick_with_an_exhausted_counter_carries_into_the_wall_clock() {
        let clock = HybridLogicalClock {
            wall_ms: 100,
            counter: u32::MAX,
        };
        let next = clock.tick(100);
        assert_eq!(next.wall_ms, 101, "the carry moves into the wall clock");
        assert_eq!(next.counter, 0);
        assert!(next > clock, "an exhausted counter must not wrap backwards");
    }

    #[test]
    fn observing_an_exhausted_peer_counter_carries_into_the_wall_clock() {
        let local = HybridLogicalClock {
            wall_ms: 100,
            counter: 0,
        };
        let remote = HybridLogicalClock {
            wall_ms: 500,
            counter: u32::MAX,
        };
        let next = local.observe(&remote, 120);
        assert_eq!(next.wall_ms, 501);
        assert_eq!(next.counter, 0);
        assert!(next > remote, "receipt still happens after send");
    }

    #[test]
    fn observing_with_our_own_counter_exhausted_carries_into_the_wall_clock() {
        let local = HybridLogicalClock {
            wall_ms: 900,
            counter: u32::MAX,
        };
        let remote = HybridLogicalClock {
            wall_ms: 100,
            counter: 0,
        };
        let next = local.observe(&remote, 900);
        assert_eq!(next.wall_ms, 901);
        assert_eq!(next.counter, 0);
        assert!(next > local);
    }

    #[test]
    fn observing_a_shared_millisecond_with_both_counters_exhausted_still_advances() {
        let local = HybridLogicalClock {
            wall_ms: 700,
            counter: u32::MAX,
        };
        let remote = HybridLogicalClock {
            wall_ms: 700,
            counter: u32::MAX,
        };
        let next = local.observe(&remote, 700);
        assert_eq!(next.wall_ms, 701);
        assert_eq!(next.counter, 0);
        assert!(next > local && next > remote);
    }

    #[test]
    fn a_fully_saturated_clock_holds_rather_than_regressing() {
        let clock = HybridLogicalClock {
            wall_ms: u64::MAX,
            counter: u32::MAX,
        };
        let next = clock.tick(u64::MAX);
        assert_eq!(
            next, clock,
            "with no later value representable, standing still beats going backwards"
        );
        assert_eq!(clock.observe(&clock, u64::MAX), clock);
    }

    #[test]
    fn observing_when_the_wall_clock_leads_both_resets_the_counter() {
        let local = HybridLogicalClock {
            wall_ms: 100,
            counter: 7,
        };
        let remote = HybridLogicalClock {
            wall_ms: 200,
            counter: 9,
        };
        let next = local.observe(&remote, 5_000);
        assert_eq!(next.wall_ms, 5_000);
        assert_eq!(next.counter, 0);
    }
}

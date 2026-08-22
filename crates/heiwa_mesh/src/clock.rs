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
            Self {
                wall_ms: self.wall_ms,
                counter: self.counter + 1,
            }
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
            self.counter.max(remote.counter) + 1
        } else if self.wall_ms > remote.wall_ms {
            self.counter + 1
        } else {
            remote.counter + 1
        };
        Self {
            wall_ms: highest,
            counter,
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

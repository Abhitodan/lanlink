//! Adaptive bitrate from observable signals.
//!
//! Inputs, sampled once per second, are things the host can measure:
//! frames discarded because the network could not keep up, QUIC's smoothed
//! RTT against the minimum RTT seen, and packets QUIC declared lost.
//!
//! * Any discarded frame, or RTT well above the path minimum (queueing),
//!   or more than 2 % packet loss → multiplicative decrease to 70 %.
//! * Three clean seconds in a row → 10 % increase, up to the plan's target.
//!
//! This is deliberately simple and conservative; its constants are starting
//! points to be tuned from the measurements in `docs/performance.md`.

#[derive(Debug, Clone, Copy, Default)]
pub struct Sample {
    pub dropped_before_send: u32,
    pub rtt_us: u32,
    pub min_rtt_us: u32,
    pub lost_packets: u64,
    pub sent_packets: u64,
}

#[derive(Debug)]
pub struct BitrateController {
    target: u32,
    floor: u32,
    current: u32,
    clean_seconds: u32,
}

/// Extra RTT over the path minimum treated as congestion.
const QUEUE_RTT_US: u32 = 40_000;

impl BitrateController {
    pub fn new(target: u32, floor: u32) -> Self {
        BitrateController {
            target,
            floor: floor.min(target),
            current: target,
            clean_seconds: 0,
        }
    }

    pub fn current(&self) -> u32 {
        self.current
    }

    /// Feed one second of measurements. Returns the new bitrate if it
    /// changed.
    pub fn update(&mut self, s: Sample) -> Option<u32> {
        let queueing = s.min_rtt_us > 0
            && s.rtt_us > s.min_rtt_us.saturating_add(QUEUE_RTT_US)
            && s.rtt_us > s.min_rtt_us.saturating_mul(2);
        let lossy = s.sent_packets >= 50 && s.lost_packets * 50 > s.sent_packets;
        let before = self.current;
        if s.dropped_before_send > 0 || queueing || lossy {
            self.clean_seconds = 0;
            self.current = ((self.current as u64 * 7 / 10) as u32).max(self.floor);
        } else {
            self.clean_seconds += 1;
            if self.clean_seconds >= 3 && self.current < self.target {
                self.clean_seconds = 0;
                self.current = ((self.current as u64 * 11 / 10) as u32).min(self.target);
            }
        }
        (self.current != before).then_some(self.current)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clean() -> Sample {
        Sample {
            dropped_before_send: 0,
            rtt_us: 6000,
            min_rtt_us: 5000,
            lost_packets: 0,
            sent_packets: 1000,
        }
    }

    #[test]
    fn steady_network_keeps_target() {
        let mut c = BitrateController::new(10_000_000, 1_000_000);
        for _ in 0..10 {
            assert_eq!(c.update(clean()), None);
        }
        assert_eq!(c.current(), 10_000_000);
    }

    #[test]
    fn drops_cut_bitrate_down_to_floor_then_recover_slowly() {
        let mut c = BitrateController::new(10_000_000, 2_000_000);
        let bad = Sample {
            dropped_before_send: 3,
            ..clean()
        };
        assert_eq!(c.update(bad), Some(7_000_000));
        for _ in 0..10 {
            c.update(bad);
        }
        assert_eq!(c.current(), 2_000_000);
        // Two clean seconds: no change yet; third: +10 %.
        assert_eq!(c.update(clean()), None);
        assert_eq!(c.update(clean()), None);
        assert_eq!(c.update(clean()), Some(2_200_000));
        for _ in 0..200 {
            c.update(clean());
        }
        assert_eq!(c.current(), 10_000_000);
    }

    #[test]
    fn queueing_delay_and_loss_count_as_congestion() {
        let mut c = BitrateController::new(10_000_000, 1_000_000);
        let queued = Sample {
            rtt_us: 60_000,
            min_rtt_us: 5_000,
            ..clean()
        };
        assert!(c.update(queued).is_some());
        let mut c = BitrateController::new(10_000_000, 1_000_000);
        // Small absolute RTT rise on a fast path is not congestion.
        assert!(c
            .update(Sample {
                rtt_us: 12_000,
                min_rtt_us: 5_000,
                ..clean()
            })
            .is_none());
        let lossy = Sample {
            lost_packets: 30,
            sent_packets: 1000,
            ..clean()
        };
        assert!(c.update(lossy).is_some());
        // Too few packets to judge loss.
        let mut c = BitrateController::new(10_000_000, 1_000_000);
        assert!(c
            .update(Sample {
                lost_packets: 2,
                sent_packets: 10,
                ..clean()
            })
            .is_none());
    }
}

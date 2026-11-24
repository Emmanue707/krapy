use std::time::{Duration, Instant};

use crate::preset::Quality;

const FLOOR_FPS: u8 = 15;
const FPS_LOW_BITRATE: u32 = 1_500_000;
const FPS_HIGH_BITRATE: u32 = 2_500_000;
const STEP_UP_AFTER: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NetStats {
    pub rtt_ms: f32,
    pub jitter_ms: f32,
    pub loss: f32,
    pub late_frames: u32,
}

#[derive(Debug)]
pub struct Adaptive {
    current: Quality,
    ceiling: Quality,
    baseline_rtt_ms: Option<f32>,
    clean_since: Option<Instant>,
    loss_windows: u8,
    rtt_spike_windows: u8,
}

impl Adaptive {
    pub fn new(ceiling: Quality) -> Self {
        Self {
            current: ceiling,
            ceiling,
            baseline_rtt_ms: None,
            clean_since: None,
            loss_windows: 0,
            rtt_spike_windows: 0,
        }
    }

    pub fn set_ceiling(&mut self, ceiling: Quality) {
        self.ceiling = ceiling;
        self.current = ceiling;
        self.clean_since = None;
        self.loss_windows = 0;
        self.rtt_spike_windows = 0;
    }

    pub fn quality(&self) -> Quality {
        self.current
    }

    fn floor_bitrate(&self) -> u32 {
        self.ceiling.bitrate / 2
    }

    fn update_baseline(&mut self, rtt_ms: f32) {
        match self.baseline_rtt_ms {
            None => self.baseline_rtt_ms = Some(rtt_ms),
            Some(base) if rtt_ms < base => self.baseline_rtt_ms = Some(rtt_ms),
            Some(base) => {
                let drift = (rtt_ms - base) * 0.05;
                self.baseline_rtt_ms = Some(base + drift);
            }
        }
    }

    fn rtt_spike(&self, stats: &NetStats) -> bool {
        let Some(base) = self.baseline_rtt_ms else {
            return false;
        };
        let threshold = (base * 2.0).max(base + 40.0);
        stats.rtt_ms > threshold
    }

    fn should_step_down(&mut self, stats: &NetStats) -> bool {
        if stats.loss > 0.02 {
            self.loss_windows = self.loss_windows.saturating_add(1);
        } else {
            self.loss_windows = 0;
        }
        if self.loss_windows >= 2 {
            return true;
        }
        if self.rtt_spike(stats) {
            self.rtt_spike_windows = self.rtt_spike_windows.saturating_add(1);
        } else {
            self.rtt_spike_windows = 0;
        }
        self.rtt_spike_windows >= 2
    }

    fn apply_fps_hysteresis(&mut self) {
        if self.current.bitrate < FPS_LOW_BITRATE {
            self.current.fps = 30;
        } else if self.current.bitrate > FPS_HIGH_BITRATE {
            self.current.fps = self.ceiling.fps;
        }
    }

    fn clamp_floor(&mut self) {
        let floor = self.floor_bitrate();
        if self.current.bitrate < floor {
            self.current.bitrate = floor;
        }
        if self.current.fps < FLOOR_FPS {
            self.current.fps = FLOOR_FPS;
        }
    }

    fn clamp_ceiling(&mut self) {
        if self.current.bitrate > self.ceiling.bitrate {
            self.current.bitrate = self.ceiling.bitrate;
        }
        if self.current.fps > self.ceiling.fps {
            self.current.fps = self.ceiling.fps;
        }
        if self.current.max_size > self.ceiling.max_size {
            self.current.max_size = self.ceiling.max_size;
        }
    }

    pub fn on_window(&mut self, stats: NetStats, now: Instant) -> Option<Quality> {
        let before = self.current;
        self.update_baseline(stats.rtt_ms);

        if self.should_step_down(&stats) {
            self.current.bitrate = ((self.current.bitrate as f64) * 0.85) as u32;
            self.apply_fps_hysteresis();
            self.clamp_floor();
            self.clean_since = None;
            self.loss_windows = 0;
            self.rtt_spike_windows = 0;
        } else {
            match self.clean_since {
                None => self.clean_since = Some(now),
                Some(since) if now.duration_since(since) >= STEP_UP_AFTER => {
                    let next = ((self.current.bitrate as f64) * 1.1) as u32;
                    if next > self.current.bitrate {
                        self.current.bitrate = next;
                        self.apply_fps_hysteresis();
                        self.clamp_ceiling();
                        self.clamp_floor();
                        self.clean_since = Some(now);
                    }
                }
                Some(_) => {}
            }
        }

        self.clamp_ceiling();
        self.clamp_floor();
        self.apply_fps_hysteresis();

        if self.current != before {
            Some(self.current)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::preset::Preset;

    fn ceiling() -> Quality {
        Preset::Balanced.quality()
    }

    fn clean_stats(rtt: f32) -> NetStats {
        NetStats {
            rtt_ms: rtt,
            jitter_ms: 0.0,
            loss: 0.0,
            late_frames: 0,
        }
    }

    #[test]
    fn down_after_two_lossy_windows() {
        let mut a = Adaptive::new(ceiling());
        let t0 = Instant::now();
        let _ = a.on_window(clean_stats(20.0), t0);
        let q1 = a.on_window(
            NetStats {
                rtt_ms: 20.0,
                jitter_ms: 0.0,
                loss: 0.03,
                late_frames: 0,
            },
            t0,
        );
        assert!(q1.is_none());
        let q2 = a.on_window(
            NetStats {
                rtt_ms: 20.0,
                jitter_ms: 0.0,
                loss: 0.03,
                late_frames: 0,
            },
            t0 + Duration::from_millis(500),
        );
        assert!(q2.is_some());
        assert!(a.quality().bitrate < ceiling().bitrate);
    }

    #[test]
    fn down_after_two_rtt_spike_windows() {
        let mut a = Adaptive::new(ceiling());
        let t0 = Instant::now();
        let _ = a.on_window(clean_stats(20.0), t0);
        let spike = NetStats {
            rtt_ms: 200.0,
            jitter_ms: 0.0,
            loss: 0.0,
            late_frames: 0,
        };
        assert!(a.on_window(spike, t0).is_none());
        assert!(
            a.on_window(spike, t0 + Duration::from_millis(500))
                .is_some()
        );
    }

    #[test]
    fn up_slow_requires_two_seconds() {
        let mut a = Adaptive::new(ceiling());
        let t0 = Instant::now();
        let _ = a.on_window(
            NetStats {
                rtt_ms: 20.0,
                jitter_ms: 0.0,
                loss: 0.05,
                late_frames: 0,
            },
            t0,
        );
        let _ = a.on_window(
            NetStats {
                rtt_ms: 20.0,
                jitter_ms: 0.0,
                loss: 0.05,
                late_frames: 0,
            },
            t0 + Duration::from_millis(500),
        );
        let after_down = a.quality().bitrate;
        let t1 = t0 + Duration::from_secs(1);
        let _ = a.on_window(clean_stats(20.0), t1);
        let q1 = a.on_window(clean_stats(20.0), t1 + Duration::from_secs(1));
        assert!(q1.is_none());
        let q2 = a.on_window(clean_stats(20.0), t1 + Duration::from_secs(2));
        assert!(q2.is_some());
        assert!(a.quality().bitrate > after_down);
    }

    #[test]
    fn cap_at_ceiling() {
        let mut a = Adaptive::new(ceiling());
        let t0 = Instant::now();
        for i in 0..20 {
            let _ = a.on_window(clean_stats(10.0), t0 + Duration::from_secs(i * 5));
        }
        assert!(a.quality().bitrate <= ceiling().bitrate);
    }

    #[test]
    fn floor_half_preset() {
        let mut a = Adaptive::new(ceiling());
        let t0 = Instant::now();
        let _ = a.on_window(clean_stats(30.0), t0);
        for _ in 0..50 {
            let _ = a.on_window(
                NetStats {
                    rtt_ms: 500.0,
                    jitter_ms: 50.0,
                    loss: 0.5,
                    late_frames: 1,
                },
                t0,
            );
            let _ = a.on_window(
                NetStats {
                    rtt_ms: 500.0,
                    jitter_ms: 50.0,
                    loss: 0.5,
                    late_frames: 1,
                },
                t0,
            );
        }
        assert!(a.quality().bitrate >= ceiling().bitrate / 2);
        assert!(a.quality().fps >= FLOOR_FPS);
    }

    #[test]
    fn fps_hysteresis() {
        let mut a = Adaptive::new(ceiling());
        let t0 = Instant::now();
        let _ = a.on_window(clean_stats(20.0), t0);
        for _ in 0..30 {
            let bad = NetStats {
                rtt_ms: 20.0,
                jitter_ms: 0.0,
                loss: 0.1,
                late_frames: 0,
            };
            let _ = a.on_window(bad, t0);
            let _ = a.on_window(bad, t0);
        }
        assert_eq!(a.quality().bitrate, ceiling().bitrate / 2);

        for i in 0..4 {
            let _ = a.on_window(clean_stats(20.0), t0 + Duration::from_secs(2 * (i + 1)));
        }
        assert!(a.quality().bitrate > ceiling().bitrate / 2);
    }
}

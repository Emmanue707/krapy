use std::time::{Duration, Instant};

use crate::preset::Quality;

const FLOOR_BITRATE: u32 = 500_000;
const FLOOR_FPS: u8 = 15;
const FPS_LOW_BITRATE: u32 = 1_500_000;
const FPS_HIGH_BITRATE: u32 = 2_500_000;
const STEP_UP_AFTER: Duration = Duration::from_secs(4);

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
}

impl Adaptive {
    pub fn new(ceiling: Quality) -> Self {
        Self {
            current: ceiling,
            ceiling,
            baseline_rtt_ms: None,
            clean_since: None,
        }
    }

    pub fn quality(&self) -> Quality {
        self.current
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

    fn should_step_down(&self, stats: &NetStats) -> bool {
        if stats.loss > 0.02 {
            return true;
        }
        if stats.jitter_ms > 30.0 {
            return true;
        }
        if stats.late_frames > 0 {
            return true;
        }
        let Some(base) = self.baseline_rtt_ms else {
            return false;
        };
        let threshold = (base * 2.0).max(base + 40.0);
        stats.rtt_ms > threshold
    }

    fn apply_fps_hysteresis(&mut self) {
        if self.current.bitrate < FPS_LOW_BITRATE {
            self.current.fps = 30;
        } else if self.current.bitrate > FPS_HIGH_BITRATE {
            self.current.fps = self.ceiling.fps;
        }
    }

    fn clamp_floor(&mut self) {
        if self.current.bitrate < FLOOR_BITRATE {
            self.current.bitrate = FLOOR_BITRATE;
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
            self.current.bitrate = ((self.current.bitrate as f64) * 0.7) as u32;
            self.apply_fps_hysteresis();
            self.clamp_floor();
            self.clean_since = None;
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
    fn down_fast_on_loss() {
        let mut a = Adaptive::new(ceiling());
        let t0 = Instant::now();
        let _ = a.on_window(clean_stats(20.0), t0);
        let q = a.on_window(
            NetStats {
                rtt_ms: 20.0,
                jitter_ms: 0.0,
                loss: 0.03,
                late_frames: 0,
            },
            t0,
        );
        assert!(q.is_some());
        assert!(a.quality().bitrate < ceiling().bitrate);
    }

    #[test]
    fn up_slow_requires_four_seconds() {
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
        let after_down = a.quality().bitrate;
        let t1 = t0 + Duration::from_secs(1);
        let _ = a.on_window(clean_stats(20.0), t1);
        let q1 = a.on_window(clean_stats(20.0), t1 + Duration::from_secs(2));
        assert!(q1.is_none());
        let q2 = a.on_window(clean_stats(20.0), t1 + Duration::from_secs(4));
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
    fn floor_bitrate() {
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
        }
        assert!(a.quality().bitrate >= FLOOR_BITRATE);
        assert!(a.quality().fps >= FLOOR_FPS);
    }

    #[test]
    fn fps_hysteresis() {
        let mut a = Adaptive::new(ceiling());
        let t0 = Instant::now();
        let _ = a.on_window(clean_stats(20.0), t0);
        for _ in 0..30 {
            let _ = a.on_window(
                NetStats {
                    rtt_ms: 20.0,
                    jitter_ms: 0.0,
                    loss: 0.1,
                    late_frames: 0,
                },
                t0,
            );
        }
        assert!(a.quality().bitrate < FPS_LOW_BITRATE);
        assert_eq!(a.quality().fps, 30);

        for _ in 0..10 {
            let _ = a.on_window(clean_stats(20.0), t0 + Duration::from_secs(60));
        }
        if a.quality().bitrate > FPS_HIGH_BITRATE {
            assert_eq!(a.quality().fps, ceiling().fps);
        }
    }
}
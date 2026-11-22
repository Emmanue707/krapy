use krapy_core::adaptive::NetStats;
use std::time::{Duration, Instant};

const WINDOW: Duration = Duration::from_millis(500);
const EWMA_ALPHA: f32 = 0.125;

#[derive(Debug, Default)]
pub struct NetWindow {
    window_start: Option<Instant>,
    rtt_ewma: Option<f32>,
    jitter: f32,
    prev_capture: Option<u64>,
    prev_arrival_us: Option<u64>,
    frames_completed: u32,
    frames_dropped: u32,
    late_frames: u32,
}

impl NetWindow {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn note_ping_rtt(&mut self, rtt: Duration) {
        let ms = rtt.as_secs_f32() * 1000.0;
        self.rtt_ewma = Some(match self.rtt_ewma {
            None => ms,
            Some(prev) => prev + EWMA_ALPHA * (ms - prev),
        });
    }

    pub fn note_video_arrival(&mut self, capture_us: u64, arrival_us: u64) {
        if let (Some(pc), Some(pa)) = (self.prev_capture, self.prev_arrival_us) {
            let d = (arrival_us as i64 - pa as i64) - (capture_us as i64 - pc as i64);
            let d = d.unsigned_abs() as f32;
            self.jitter += (d - self.jitter) / 16.0;
        }
        self.prev_capture = Some(capture_us);
        self.prev_arrival_us = Some(arrival_us);
    }

    pub fn note_frame_completed(&mut self) {
        self.frames_completed += 1;
    }

    pub fn note_frame_dropped(&mut self) {
        self.frames_dropped += 1;
    }

    pub fn note_late_frame(&mut self) {
        self.late_frames += 1;
    }

    pub fn rtt_ms(&self) -> f32 {
        self.rtt_ewma.unwrap_or(0.0)
    }

    pub fn tick(&mut self, now: Instant) -> Option<NetStats> {
        let start = match self.window_start {
            Some(s) => s,
            None => {
                self.window_start = Some(now);
                return None;
            }
        };
        if now.duration_since(start) < WINDOW {
            return None;
        }
        let total = self.frames_completed + self.frames_dropped;
        let loss = if total == 0 {
            0.0
        } else {
            self.frames_dropped as f32 / total as f32
        };
        let stats = NetStats {
            rtt_ms: self.rtt_ewma.unwrap_or(0.0),
            jitter_ms: self.jitter / 1000.0,
            loss,
            late_frames: self.late_frames,
        };
        self.window_start = Some(now);
        self.frames_completed = 0;
        self.frames_dropped = 0;
        self.late_frames = 0;
        Some(stats)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn synthetic_rtt_ewma_and_jitter() {
        let mut w = NetWindow::new();
        let t0 = Instant::now();
        w.window_start = Some(t0);
        w.note_ping_rtt(Duration::from_millis(20));
        w.note_ping_rtt(Duration::from_millis(40));
        w.note_video_arrival(1_000_000, 1_000_050);
        w.note_video_arrival(1_033_000, 1_033_120);
        w.frames_completed = 10;
        w.frames_dropped = 1;
        w.late_frames = 2;
        let stats = w.tick(t0 + WINDOW).expect("window");
        assert!(stats.rtt_ms > 0.0);
        assert!(stats.jitter_ms >= 0.0);
        assert!((stats.loss - 1.0 / 11.0).abs() < 0.01);
        assert_eq!(stats.late_frames, 2);
    }
}

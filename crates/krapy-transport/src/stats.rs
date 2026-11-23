use krapy_core::adaptive::NetStats;
use std::collections::VecDeque;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PhoneStatsSnap {
    pub enc_avg_us: u32,
    pub enc_max_us: u32,
    pub fps: u8,
    pub bitrate: u32,
}

#[derive(Debug, Default)]
pub struct ClockSync {
    samples: VecDeque<i64>,
}

impl ClockSync {
    pub fn note_pong(&mut self, t_send_us: u64, phone_us: u64, now_us: u64) {
        if now_us < t_send_us {
            return;
        }
        let rtt = now_us - t_send_us;
        let offset = phone_us as i64 - (t_send_us as i64 + rtt as i64 / 2);
        if self.samples.len() >= 16 {
            self.samples.pop_front();
        }
        self.samples.push_back(offset);
    }

    pub fn median_offset_us(&self) -> Option<i64> {
        if self.samples.is_empty() {
            return None;
        }
        let mut v: Vec<i64> = self.samples.iter().copied().collect();
        v.sort_unstable();
        Some(v[v.len() / 2])
    }
}

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
    total_dropped: u64,
    last: Option<NetStats>,
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
        self.late_frames += 1;
        self.total_dropped += 1;
    }

    pub fn total_dropped(&self) -> u64 {
        self.total_dropped
    }

    pub fn note_late_frame(&mut self) {
        self.late_frames += 1;
    }

    pub fn last(&self) -> Option<NetStats> {
        self.last
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
        self.last = Some(stats);
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

    #[test]
    fn reassembler_drops_feed_loss_and_late() {
        use crate::reassembly::Reassembler;
        use crate::wire::Datagram;
        use std::borrow::Cow;

        fn frag(id: u32, idx: u16, count: u16, flags: u8, data: &[u8]) -> Datagram<'static> {
            Datagram::Video {
                frame_id: id,
                idx,
                count,
                flags,
                capture_us: id as u64 * 1_000,
                data: Cow::Owned(data.to_vec()),
            }
        }

        let mut r = Reassembler::new();
        let mut w = NetWindow::new();
        w.window_start = Some(Instant::now());

        let prev = r.stats().frames_dropped;
        let _ = r.ingest(&frag(1, 0, 2, 0, b"a"), 1000);
        let _ = r.ingest(&frag(2, 0, 2, 0, b"x"), 2000);
        let _ = r.ingest(&frag(2, 1, 2, 0, b"y"), 2100);
        let new = r.stats().frames_dropped;
        for _ in 0..(new - prev) {
            w.note_frame_dropped();
        }
        assert_eq!(w.total_dropped(), 2);

        w.frames_completed = 4;
        let stats = w.tick(Instant::now() + WINDOW).expect("window");
        assert!((stats.loss - 2.0 / 6.0).abs() < 0.01);
        assert_eq!(stats.late_frames, 2);
    }
}

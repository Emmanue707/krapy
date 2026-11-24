use std::collections::VecDeque;
use std::sync::{Condvar, Mutex};
use std::time::Duration;

use crate::reassembly::{CompleteFrame, FLAG_CODEC_CONFIG, FLAG_KEYFRAME};

// decoder backlog: past this we skip ahead to the next keyframe instead of falling behind
const MAX_BACKLOG: usize = 3;

#[derive(Default)]
struct State {
    q: VecDeque<CompleteFrame>,
    need_key: bool,
    skipped: u64,
}

#[derive(Default)]
pub struct FrameQueue {
    s: Mutex<State>,
    ready: Condvar,
}

impl FrameQueue {
    pub fn new() -> Self {
        Self::default()
    }

    // returns true when the sender should be asked for a keyframe
    pub fn push(&self, f: CompleteFrame) -> bool {
        let Ok(mut s) = self.s.lock() else {
            return false;
        };
        let key = f.flags & FLAG_KEYFRAME != 0;
        if key {
            // a keyframe makes everything queued before it useless
            let stale = s.q.iter().filter(|x| !is_config(x)).count() as u64;
            s.skipped += stale;
            s.q.retain(is_config);
            s.need_key = false;
            s.q.push_back(f);
            self.ready.notify_one();
            return false;
        }
        if is_config(&f) {
            s.q.push_back(f);
            self.ready.notify_one();
            return false;
        }
        if s.need_key {
            s.skipped += 1;
            return true;
        }
        if s.q.len() >= MAX_BACKLOG {
            let stale = s.q.iter().filter(|x| !is_config(x)).count() as u64 + 1;
            s.skipped += stale;
            s.q.retain(is_config);
            s.need_key = true;
            return true;
        }
        s.q.push_back(f);
        self.ready.notify_one();
        false
    }

    pub fn pop(&self) -> Option<CompleteFrame> {
        self.s.lock().ok().and_then(|mut s| s.q.pop_front())
    }

    pub fn pop_wait(&self, timeout: Duration) -> Option<CompleteFrame> {
        let s = self.s.lock().ok()?;
        let (mut s, _) = self
            .ready
            .wait_timeout_while(s, timeout, |s| s.q.is_empty())
            .ok()?;
        s.q.pop_front()
    }

    // the decoder lost its reference: drop until the next keyframe
    pub fn broken(&self) {
        if let Ok(mut s) = self.s.lock() {
            let stale = s.q.iter().filter(|x| !is_config(x)).count() as u64;
            s.skipped += stale;
            s.q.retain(is_config);
            s.need_key = true;
        }
    }

    pub fn waiting_for_key(&self) -> bool {
        self.s.lock().map(|s| s.need_key).unwrap_or(false)
    }

    pub fn skipped(&self) -> u64 {
        self.s.lock().map(|s| s.skipped).unwrap_or(0)
    }

    pub fn len(&self) -> usize {
        self.s.lock().map(|s| s.q.len()).unwrap_or(0)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

fn is_config(f: &CompleteFrame) -> bool {
    f.flags & FLAG_CODEC_CONFIG != 0 && f.flags & FLAG_KEYFRAME == 0
}

const MAX_AUDIO_AGE_US: u64 = 80_000;

struct AudioPacket {
    seq: u32,
    pts_us: u64,
    data: Vec<u8>,
}

#[derive(Default)]
struct AudioState {
    q: VecDeque<AudioPacket>,
}

pub struct AudioQueue {
    s: Mutex<AudioState>,
    ready: Condvar,
}

impl AudioQueue {
    pub fn new() -> Self {
        Self {
            s: Mutex::new(AudioState::default()),
            ready: Condvar::new(),
        }
    }

    pub fn push(&self, seq: u32, pts_us: u64, data: &[u8]) {
        let Ok(mut s) = self.s.lock() else {
            return;
        };
        if data.len() > crate::wire::MAX_AUDIO_DATA {
            return;
        }
        let mut owned = Vec::with_capacity(data.len());
        owned.extend_from_slice(data);
        s.q.push_back(AudioPacket {
            seq,
            pts_us,
            data: owned,
        });
        self.ready.notify_one();
        while s.q.len() > 1 {
            let oldest = s.q.front().expect("f");
            let newest = s.q.back().expect("b");
            if newest.pts_us.saturating_sub(oldest.pts_us) > MAX_AUDIO_AGE_US {
                s.q.pop_front();
            } else {
                break;
            }
        }
        while s.q.len() > 24 {
            s.q.pop_front();
        }
    }

    pub fn pop(&self, out: &mut [u8]) -> Option<(u32, u64, usize)> {
        let Ok(mut s) = self.s.lock() else {
            return None;
        };
        let pkt = s.q.pop_front()?;
        let n = pkt.data.len().min(out.len());
        out[..n].copy_from_slice(&pkt.data[..n]);
        Some((pkt.seq, pkt.pts_us, n))
    }

    pub fn pop_wait(&self, timeout: Duration, out: &mut [u8]) -> Option<(u32, u64, usize)> {
        let s = self.s.lock().ok()?;
        let (mut s, _) = self
            .ready
            .wait_timeout_while(s, timeout, |s| s.q.is_empty())
            .ok()?;
        let pkt = s.q.pop_front()?;
        let n = pkt.data.len().min(out.len());
        out[..n].copy_from_slice(&pkt.data[..n]);
        Some((pkt.seq, pkt.pts_us, n))
    }
}

impl Default for AudioQueue {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    fn f(id: u32, flags: u8) -> CompleteFrame {
        CompleteFrame {
            frame_id: id,
            flags,
            capture_us: id as u64,
            first_arrival_us: id as u64,
            completed_us: id as u64 + 100,
            data: vec![id as u8],
            received: Instant::now(),
        }
    }

    fn ids(q: &FrameQueue) -> Vec<u32> {
        let mut v = Vec::new();
        while let Some(x) = q.pop() {
            v.push(x.frame_id);
        }
        v
    }

    #[test]
    fn keeps_order_when_decoder_keeps_up() {
        let q = FrameQueue::new();
        assert!(!q.push(f(1, FLAG_KEYFRAME)));
        assert!(!q.push(f(2, 0)));
        assert_eq!(ids(&q), vec![1, 2]);
        assert!(!q.push(f(3, 0)));
        assert_eq!(ids(&q), vec![3]);
    }

    #[test]
    fn config_is_never_lost() {
        let q = FrameQueue::new();
        q.push(f(1, FLAG_CODEC_CONFIG));
        q.push(f(2, FLAG_KEYFRAME));
        assert_eq!(ids(&q), vec![1, 2]);
    }

    #[test]
    fn keyframe_flushes_backlog() {
        let q = FrameQueue::new();
        q.push(f(1, FLAG_KEYFRAME));
        q.push(f(2, 0));
        q.push(f(3, FLAG_KEYFRAME));
        assert_eq!(ids(&q), vec![3]);
        assert_eq!(q.skipped(), 2);
    }

    #[test]
    fn backlog_skips_to_next_keyframe() {
        let q = FrameQueue::new();
        q.push(f(1, FLAG_KEYFRAME));
        q.push(f(2, 0));
        q.push(f(3, 0));
        assert!(q.push(f(4, 0)));
        assert!(q.waiting_for_key());
        assert!(q.push(f(5, 0)));
        assert!(q.is_empty());
        assert!(!q.push(f(6, FLAG_KEYFRAME)));
        assert!(!q.waiting_for_key());
        assert_eq!(ids(&q), vec![6]);
    }

    #[test]
    fn broken_decoder_waits_for_keyframe() {
        let q = FrameQueue::new();
        q.push(f(1, FLAG_KEYFRAME));
        q.broken();
        assert!(q.push(f(2, 0)));
        assert_eq!(ids(&q), Vec::<u32>::new());
        q.push(f(3, FLAG_KEYFRAME));
        assert_eq!(ids(&q), vec![3]);
    }
}

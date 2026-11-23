use std::collections::VecDeque;
use std::sync::Mutex;

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
            return false;
        }
        if is_config(&f) {
            s.q.push_back(f);
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
        false
    }

    pub fn pop(&self) -> Option<CompleteFrame> {
        self.s.lock().ok().and_then(|mut s| s.q.pop_front())
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    fn f(id: u32, flags: u8) -> CompleteFrame {
        CompleteFrame {
            frame_id: id,
            flags,
            capture_us: id as u64,
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

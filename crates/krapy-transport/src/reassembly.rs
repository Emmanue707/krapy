use crate::wire::Datagram;
use std::time::Instant;

pub const FLAG_KEYFRAME: u8 = 1;
pub const FLAG_CODEC_CONFIG: u8 = 2;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompleteFrame {
    pub frame_id: u32,
    pub flags: u8,
    pub capture_us: u64,
    pub first_arrival_us: u64,
    pub completed_us: u64,
    pub data: Vec<u8>,
    pub received: Instant,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct ReassemblyStats {
    pub frames_ok: u64,
    pub frames_dropped: u64,
    pub fragments_received: u64,
    pub bytes: u64,
}

struct Partial {
    frame_id: u32,
    count: u16,
    flags: u8,
    capture_us: u64,
    first_arrival_us: u64,
    parts: Vec<Option<Vec<u8>>>,
    received: u16,
}

pub struct Reassembler {
    last_completed: Option<u32>,
    partials: Vec<Partial>,
    need_keyframe: bool,
    stats: ReassemblyStats,
    out_buf: Vec<u8>,
}

impl Reassembler {
    pub fn new() -> Self {
        Self {
            last_completed: None,
            partials: Vec::new(),
            need_keyframe: false,
            stats: ReassemblyStats::default(),
            out_buf: Vec::with_capacity(4096),
        }
    }

    pub fn stats(&self) -> ReassemblyStats {
        self.stats
    }

    pub fn keyframe_wanted(&self) -> bool {
        self.need_keyframe
    }

    pub fn clear_keyframe_wanted(&mut self) {
        self.need_keyframe = false;
    }

    pub fn ingest(&mut self, dg: &Datagram<'_>, arrival_us: u64) -> Option<CompleteFrame> {
        let Datagram::Video {
            frame_id,
            idx,
            count,
            flags,
            capture_us,
            data,
        } = dg
        else {
            return None;
        };
        if *count == 0 {
            return None;
        }
        if *idx as usize >= *count as usize {
            return None;
        }
        self.stats.fragments_received += 1;
        self.stats.bytes += data.len() as u64;

        if let Some(last) = self.last_completed
            && *frame_id < last
        {
            return None;
        }

        let pos = self.partials.iter().position(|p| p.frame_id == *frame_id);
        if pos.is_none() {
            if let Some(last) = self.last_completed
                && *frame_id < last
            {
                return None;
            }
            while self.partials.len() >= 3 {
                let dropped = self.partials.remove(0);
                self.stats.frames_dropped += 1;
                if dropped.flags & FLAG_CODEC_CONFIG == 0 || dropped.flags & FLAG_KEYFRAME != 0 {
                    self.need_keyframe = true;
                }
            }
            self.partials.push(Partial {
                frame_id: *frame_id,
                count: *count,
                flags: *flags,
                capture_us: *capture_us,
                first_arrival_us: arrival_us,
                parts: vec![None; *count as usize],
                received: 0,
            });
        }

        let p = self
            .partials
            .iter_mut()
            .find(|p| p.frame_id == *frame_id)
            .expect("exists");
        if p.count != *count {
            return None;
        }
        let slot = &mut p.parts[*idx as usize];
        if slot.is_some() {
            return None;
        }
        *slot = Some(data.to_vec());
        p.received += 1;

        if p.received < p.count {
            return None;
        }

        let idx_pos = self
            .partials
            .iter()
            .position(|x| x.frame_id == *frame_id)
            .expect("p");
        let done = self.partials.remove(idx_pos);

        let older: Vec<_> = self
            .partials
            .iter()
            .filter(|p| p.frame_id < done.frame_id)
            .map(|p| p.frame_id)
            .collect();
        for oid in older {
            if let Some(i) = self.partials.iter().position(|p| p.frame_id == oid) {
                let dropped = self.partials.remove(i);
                self.stats.frames_dropped += 1;
                if dropped.flags & FLAG_CODEC_CONFIG == 0 || dropped.flags & FLAG_KEYFRAME != 0 {
                    self.need_keyframe = true;
                }
            }
        }

        self.last_completed = Some(done.frame_id);
        self.out_buf.clear();
        for part in done.parts {
            self.out_buf.extend_from_slice(&part.expect("full"));
        }
        let cap = self.out_buf.capacity();
        let data = std::mem::take(&mut self.out_buf);
        self.out_buf = Vec::with_capacity(cap.max(4096));

        let is_key = done.flags & FLAG_KEYFRAME != 0;
        let config_only = done.flags & FLAG_CODEC_CONFIG != 0 && !is_key;
        if self.need_keyframe && !is_key && !config_only {
            self.stats.frames_dropped += 1;
            self.need_keyframe = true;
            return None;
        }
        if is_key {
            self.need_keyframe = false;
        }

        self.stats.frames_ok += 1;
        Some(CompleteFrame {
            frame_id: done.frame_id,
            flags: done.flags,
            capture_us: done.capture_us,
            first_arrival_us: done.first_arrival_us,
            completed_us: arrival_us,
            data,
            received: Instant::now(),
        })
    }
}

impl Default for Reassembler {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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

    #[test]
    fn in_order() {
        let mut r = Reassembler::new();
        assert!(r.ingest(&frag(1, 0, 2, FLAG_KEYFRAME, b"a"), 0).is_none());
        let f = r
            .ingest(&frag(1, 1, 2, FLAG_KEYFRAME, b"b"), 0)
            .expect("done");
        assert_eq!(f.data, b"ab");
        assert_eq!(r.stats().frames_ok, 1);
    }

    #[test]
    fn out_of_order() {
        let mut r = Reassembler::new();
        assert!(r.ingest(&frag(1, 1, 2, FLAG_KEYFRAME, b"b"), 0).is_none());
        let f = r
            .ingest(&frag(1, 0, 2, FLAG_KEYFRAME, b"a"), 0)
            .expect("done");
        assert_eq!(f.data, b"ab");
    }

    #[test]
    fn loss_need_keyframe() {
        let mut r = Reassembler::new();
        let _ = r.ingest(&frag(1, 0, 2, 0, b"a"), 0);
        let _ = r.ingest(&frag(2, 0, 2, 0, b"x"), 0);
        let _ = r.ingest(&frag(2, 1, 2, 0, b"y"), 0);
        assert!(r.keyframe_wanted());
        assert!(r.ingest(&frag(3, 0, 1, FLAG_KEYFRAME, b"k"), 0).is_some());
        assert!(!r.keyframe_wanted());
    }

    #[test]
    fn duplicate_fragment() {
        let mut r = Reassembler::new();
        assert!(r.ingest(&frag(1, 0, 2, FLAG_KEYFRAME, b"a"), 0).is_none());
        assert!(r.ingest(&frag(1, 0, 2, FLAG_KEYFRAME, b"a"), 0).is_none());
        let f = r.ingest(&frag(1, 1, 2, FLAG_KEYFRAME, b"b"), 0).expect("d");
        assert_eq!(f.data, b"ab");
    }

    #[test]
    fn stale_fragment() {
        let mut r = Reassembler::new();
        let _ = r.ingest(&frag(1, 0, 1, FLAG_KEYFRAME, b"a"), 0);
        assert!(r.ingest(&frag(0, 0, 1, FLAG_KEYFRAME, b"z"), 0).is_none());
    }

    #[test]
    fn lost_keyframe_needs_another() {
        let mut r = Reassembler::new();
        let _ = r.ingest(&frag(1, 0, 2, FLAG_KEYFRAME, b"k"), 0);
        assert!(r.ingest(&frag(2, 0, 1, 0, b"p"), 0).is_none());
        assert!(r.keyframe_wanted());
    }

    #[test]
    fn config_passes_while_waiting_for_keyframe() {
        let mut r = Reassembler::new();
        let _ = r.ingest(&frag(1, 0, 2, 0, b"a"), 0);
        let _ = r.ingest(&frag(2, 0, 1, 0, b"b"), 0);
        assert!(r.keyframe_wanted());
        let c = r.ingest(&frag(3, 0, 1, FLAG_CODEC_CONFIG, b"sps"), 0);
        assert_eq!(c.expect("config").data, b"sps");
        assert!(r.keyframe_wanted());
    }
}

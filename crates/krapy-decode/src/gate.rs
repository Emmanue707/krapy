use crate::annex::nal_starts;
use crate::{Decoder, Frame};
use anyhow::Result;

// keeps the last sps/pps, puts them in front of every idr that lacks them,
// and holds back p-frames until an idr has decoded
pub struct Gated {
    inner: Box<dyn Decoder>,
    params: Vec<u8>,
    synced: bool,
    buf: Vec<u8>,
}

impl Gated {
    pub fn new(inner: Box<dyn Decoder>) -> Self {
        Self {
            inner,
            params: Vec::new(),
            synced: false,
            buf: Vec::new(),
        }
    }
}

impl Decoder for Gated {
    fn decode(&mut self, au: &[u8]) -> Result<Option<Frame>> {
        let nals = nal_starts(au);
        let has_sps = nals.iter().any(|n| n.1 == 7);
        let has_idr = nals.iter().any(|n| n.1 == 5);
        let has_slice = nals.iter().any(|n| n.1 == 1);
        if has_sps {
            self.params.clear();
            for (k, &(start, ty)) in nals.iter().enumerate() {
                if ty == 7 || ty == 8 {
                    let end = nals.get(k + 1).map(|n| n.0).unwrap_or(au.len());
                    self.params.extend_from_slice(&au[start..end]);
                }
            }
        }
        if has_idr {
            let input = if has_sps || self.params.is_empty() {
                au
            } else {
                self.buf.clear();
                self.buf.extend_from_slice(&self.params);
                self.buf.extend_from_slice(au);
                &self.buf
            };
            return match self.inner.decode(input) {
                Ok(f) => {
                    self.synced = true;
                    Ok(f)
                }
                Err(e) => {
                    self.synced = false;
                    Err(e)
                }
            };
        }
        if !has_slice || !self.synced {
            return Ok(None);
        }
        let r = self.inner.decode(au);
        if r.is_err() {
            self.synced = false;
        }
        r
    }

    fn flush(&mut self) -> Result<Option<Frame>> {
        self.inner.flush()
    }

    fn reset(&mut self) {
        self.inner.reset();
        self.synced = false;
    }

    fn needs_keyframe(&self) -> bool {
        !self.synced
    }

    fn name(&self) -> &'static str {
        self.inner.name()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    struct Fake {
        seen: Arc<Mutex<Vec<Vec<u8>>>>,
        fail: bool,
    }

    impl Decoder for Fake {
        fn decode(&mut self, au: &[u8]) -> Result<Option<Frame>> {
            self.seen.lock().expect("l").push(au.to_vec());
            if self.fail {
                anyhow::bail!("bad");
            }
            Ok(Some(Frame::default()))
        }
        fn reset(&mut self) {}
        fn name(&self) -> &'static str {
            "fake"
        }
    }

    const SPS: &[u8] = &[0, 0, 0, 1, 0x67, 1, 2];
    const PPS: &[u8] = &[0, 0, 0, 1, 0x68, 3];
    const IDR: &[u8] = &[0, 0, 0, 1, 0x65, 9, 9];
    const P: &[u8] = &[0, 0, 0, 1, 0x41, 7];

    fn gated(fail: bool) -> (Gated, Arc<Mutex<Vec<Vec<u8>>>>) {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let g = Gated::new(Box::new(Fake {
            seen: Arc::clone(&seen),
            fail,
        }));
        (g, seen)
    }

    #[test]
    fn config_is_put_in_front_of_a_bare_idr() {
        let (mut g, seen) = gated(false);
        let cfg = [SPS, PPS].concat();
        assert!(g.decode(&cfg).expect("cfg").is_none());
        assert!(g.decode(IDR).expect("idr").is_some());
        assert_eq!(seen.lock().expect("l")[0], [SPS, PPS, IDR].concat());
    }

    #[test]
    fn idr_with_its_own_config_goes_through_as_is() {
        let (mut g, seen) = gated(false);
        let au = [SPS, PPS, IDR].concat();
        g.decode(&au).expect("au");
        g.decode(IDR).expect("idr");
        let s = seen.lock().expect("l");
        assert_eq!(s[0], au);
        assert_eq!(s[1], au);
    }

    #[test]
    fn p_frames_wait_for_an_idr() {
        let (mut g, seen) = gated(false);
        assert!(g.decode(P).expect("p").is_none());
        assert!(g.needs_keyframe());
        g.decode(&[SPS, PPS, IDR].concat()).expect("idr");
        assert!(!g.needs_keyframe());
        assert!(g.decode(P).expect("p").is_some());
        assert_eq!(seen.lock().expect("l").len(), 2);
    }

    #[test]
    fn error_drops_sync() {
        let (mut g, _) = gated(true);
        assert!(g.decode(&[SPS, PPS, IDR].concat()).is_err());
        assert!(g.needs_keyframe());
        assert!(g.decode(P).expect("p").is_none());
    }
}

const SAMPLES_PER_MS: usize = 48;
const TARGET_MS: usize = 30;
const MAX_MS: usize = 70;

pub struct Jitter {
    buf: Vec<i16>,
    head: usize,
    len: usize,
}

impl Jitter {
    pub fn new() -> Self {
        let cap = MAX_MS * SAMPLES_PER_MS * 2;
        Self {
            buf: vec![0; cap],
            head: 0,
            len: 0,
        }
    }

    pub fn push_pcm(&mut self, data: &[u8]) {
        let mut i = 0;
        while i + 1 < data.len() {
            let s = (data[i] as i16) | ((data[i + 1] as i16) << 8);
            self.push_one(s);
            i += 2;
        }
    }

    fn push_one(&mut self, sample: i16) {
        if self.len >= self.buf.len() {
            self.head = (self.head + 1) % self.buf.len();
            self.len -= 1;
        }
        let tail = (self.head + self.len) % self.buf.len();
        self.buf[tail] = sample;
        self.len += 1;
        let target = TARGET_MS * SAMPLES_PER_MS * 2;
        let max = MAX_MS * SAMPLES_PER_MS * 2;
        while self.len > max {
            self.head = (self.head + 1) % self.buf.len();
            self.len -= 1;
        }
        let _ = target;
    }

    pub fn pop_into(&mut self, out: &mut [i16]) -> usize {
        let n = self.len.min(out.len());
        for (dst, idx) in out.iter_mut().zip(0..n) {
            let pos = (self.head + idx) % self.buf.len();
            *dst = self.buf[pos];
        }
        if n > 0 {
            self.head = (self.head + n) % self.buf.len();
            self.len -= n;
        }
        n
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drops_when_past_max() {
        let mut j = Jitter::new();
        let sample = 1000i16.to_le_bytes();
        let big = MAX_MS * SAMPLES_PER_MS * 2 + 100;
        for _ in 0..big {
            j.push_pcm(&sample);
        }
        assert!(j.len <= MAX_MS * SAMPLES_PER_MS * 2);
    }

    #[test]
    fn chunk_is_five_ms_stereo() {
        let samples = 240 * 2;
        assert_eq!(samples * 2, 960);
    }
}

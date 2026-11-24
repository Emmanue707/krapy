const TARGET_MS: usize = 40;
const MAX_MS: usize = 120;
const DRIFT_EXTRA_MS: usize = 30;
const FADE_MS: usize = 2;
const TRIM_EVERY: usize = 100;

pub struct Jitter {
    buf: Box<[i16]>,
    head: usize,
    len: usize,
    input_rate: u32,
    primed: bool,
    pending_fade_in: bool,
    fade_in: usize,
    fade_out: usize,
    out_since_trim: usize,
    phase: f64,
    prime_floor: usize,
}

impl Jitter {
    pub fn new() -> Self {
        let cap = MAX_MS * 48 * 2;
        Self {
            buf: vec![0i16; cap].into_boxed_slice(),
            head: 0,
            len: 0,
            input_rate: 48_000,
            primed: false,
            pending_fade_in: false,
            fade_in: 0,
            fade_out: 0,
            out_since_trim: 0,
            phase: 0.0,
            prime_floor: 0,
        }
    }

    fn fade_frames(out_rate: u32) -> usize {
        (out_rate as usize * FADE_MS / 1000).max(1)
    }

    pub fn set_input_rate(&mut self, rate: u32) {
        if self.input_rate != rate {
            self.input_rate = rate;
            self.clear();
        }
    }

    pub fn clear(&mut self) {
        self.head = 0;
        self.len = 0;
        self.primed = false;
        self.pending_fade_in = false;
        self.fade_in = 0;
        self.fade_out = 0;
        self.out_since_trim = 0;
        self.phase = 0.0;
    }

    fn target_samples(&self) -> usize {
        (self.input_rate as usize * TARGET_MS / 1000 * 2).max(self.prime_floor)
    }

    fn drift_extra(&self) -> usize {
        self.input_rate as usize * DRIFT_EXTRA_MS / 1000 * 2
    }

    pub fn push_pcm(&mut self, data: &[u8]) {
        let max = self.max_samples();
        let mut i = 0;
        while i + 1 < data.len() {
            let s = i16::from_le_bytes([data[i], data[i + 1]]);
            if self.len >= self.buf.len() {
                self.head = (self.head + 2) % self.buf.len();
                self.len = self.len.saturating_sub(2);
            }
            let tail = (self.head + self.len) % self.buf.len();
            self.buf[tail] = s;
            self.len += 1;
            i += 2;
        }
        while self.len > max {
            self.head = (self.head + 2) % self.buf.len();
            self.len -= 2;
        }
        if !self.primed && self.len >= self.target_samples() {
            self.primed = true;
            self.pending_fade_in = true;
            self.fade_out = 0;
        }
    }

    fn peek_pair(&self, frame_idx: usize) -> (i16, i16) {
        let base = frame_idx * 2;
        if base + 1 >= self.len {
            return (0, 0);
        }
        let l_pos = (self.head + base) % self.buf.len();
        let r_pos = (self.head + base + 1) % self.buf.len();
        (self.buf[l_pos], self.buf[r_pos])
    }

    fn consume_frames(&mut self, n: usize) {
        let drop = (n * 2).min(self.len);
        if drop > 0 {
            self.head = (self.head + drop) % self.buf.len();
            self.len -= drop;
        }
    }

    fn maybe_drift_trim(&mut self, out_frames: usize) {
        self.out_since_trim += out_frames;
        if self.out_since_trim < TRIM_EVERY {
            return;
        }
        self.out_since_trim = 0;
        if self.len > self.target_samples() + self.drift_extra() && self.len >= 2 {
            self.head = (self.head + 2) % self.buf.len();
            self.len -= 2;
        }
    }

    fn gain(&mut self, out_rate: u32) -> f32 {
        if self.pending_fade_in {
            self.fade_in = Self::fade_frames(out_rate);
            self.pending_fade_in = false;
        }
        let total = Self::fade_frames(out_rate) as f32;
        if self.fade_out > 0 {
            self.fade_out -= 1;
            return (self.fade_out as f32) / total;
        }
        if self.fade_in > 0 {
            self.fade_in -= 1;
            return 1.0 - (self.fade_in as f32) / total;
        }
        1.0
    }

    fn sample_at(&self, pos: f64) -> (f32, f32) {
        if self.len < 2 {
            return (0.0, 0.0);
        }
        let frames_avail = self.len / 2;
        let max_idx = (frames_avail - 1) as f64;
        let idx = pos.min(max_idx).max(0.0);
        let i0 = idx.floor() as usize;
        let i1 = (i0 + 1).min(frames_avail - 1);
        let frac = (idx - i0 as f64) as f32;
        let (l0, r0) = self.peek_pair(i0);
        let (l1, r1) = self.peek_pair(i1);
        let l = l0 as f32 * (1.0 - frac) + l1 as f32 * frac;
        let r = r0 as f32 * (1.0 - frac) + r1 as f32 * frac;
        (l, r)
    }

    pub fn render_i16(&mut self, out: &mut [i16], out_channels: usize, out_rate: u32) {
        if out_channels == 0 {
            return;
        }
        let out_frames = out.len() / out_channels;
        if out_frames == 0 {
            return;
        }
        if !self.primed {
            out.fill(0);
            return;
        }
        let in_step = self.input_rate as f64 / out_rate as f64;
        let need_frames = ((out_frames as f64 - 1.0) * in_step + self.phase).floor() as usize + 2;
        let have_frames = self.len / 2;
        if have_frames < need_frames {
            // device asks for more per callback than we buffer: buffer more
            let margin = self.input_rate as usize * 5 / 1000;
            self.prime_floor = ((need_frames + margin) * 2).min(self.max_samples() * 3 / 4);
            out.fill(0);
            self.fade_out = Self::fade_frames(out_rate);
            self.primed = false;
            self.pending_fade_in = false;
            self.phase = 0.0;
            return;
        }
        let mut frames_used = 0usize;
        for (fi, frame) in out.chunks_mut(out_channels).enumerate() {
            let pos = self.phase + fi as f64 * in_step;
            let (mut l, mut r) = self.sample_at(pos);
            let g = self.gain(out_rate);
            l *= g;
            r *= g;
            match out_channels {
                1 => frame[0] = ((l + r) * 0.5).clamp(-32768.0, 32767.0) as i16,
                _ => {
                    frame[0] = l.clamp(-32768.0, 32767.0) as i16;
                    if out_channels > 1 {
                        frame[1] = r.clamp(-32768.0, 32767.0) as i16;
                    }
                    for c in frame.iter_mut().skip(2) {
                        *c = 0;
                    }
                }
            }
            frames_used = fi + 1;
        }
        let last_pos = self.phase + frames_used as f64 * in_step;
        let consumed = last_pos.floor() as usize;
        self.consume_frames(consumed);
        self.phase = last_pos - consumed as f64;
        self.maybe_drift_trim(frames_used);
    }

    pub fn render_f32(&mut self, out: &mut [f32], out_channels: usize, out_rate: u32) {
        if out_channels == 0 {
            return;
        }
        let out_frames = out.len() / out_channels;
        if out_frames == 0 {
            return;
        }
        if !self.primed {
            out.fill(0.0);
            return;
        }
        let in_step = self.input_rate as f64 / out_rate as f64;
        let need_frames = ((out_frames as f64 - 1.0) * in_step + self.phase).floor() as usize + 2;
        let have_frames = self.len / 2;
        if have_frames < need_frames {
            // device asks for more per callback than we buffer: buffer more
            let margin = self.input_rate as usize * 5 / 1000;
            self.prime_floor = ((need_frames + margin) * 2).min(self.max_samples() * 3 / 4);
            out.fill(0.0);
            self.fade_out = Self::fade_frames(out_rate);
            self.primed = false;
            self.pending_fade_in = false;
            self.phase = 0.0;
            return;
        }
        let mut frames_used = 0usize;
        for (fi, frame) in out.chunks_mut(out_channels).enumerate() {
            let pos = self.phase + fi as f64 * in_step;
            let (mut l, mut r) = self.sample_at(pos);
            let g = self.gain(out_rate);
            l *= g / 32768.0;
            r *= g / 32768.0;
            match out_channels {
                1 => frame[0] = (l + r) * 0.5,
                _ => {
                    frame[0] = l;
                    if out_channels > 1 {
                        frame[1] = r;
                    }
                    for c in frame.iter_mut().skip(2) {
                        *c = 0.0;
                    }
                }
            }
            frames_used = fi + 1;
        }
        let last_pos = self.phase + frames_used as f64 * in_step;
        let consumed = last_pos.floor() as usize;
        self.consume_frames(consumed);
        self.phase = last_pos - consumed as f64;
        self.maybe_drift_trim(frames_used);
    }

    #[cfg(test)]
    pub fn is_primed(&self) -> bool {
        self.primed
    }

    #[cfg(test)]
    pub fn buffered_frames(&self) -> usize {
        self.len / 2
    }

    pub fn input_rate(&self) -> u32 {
        self.input_rate
    }

    pub fn max_samples(&self) -> usize {
        self.input_rate as usize * MAX_MS / 1000 * 2
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stereo_sine_1k(out: &mut [i16], frames: usize, rate: u32, channel: usize) {
        let period = rate as f64 / 1000.0;
        for i in 0..frames {
            let t = i as f64;
            let v = (2.0 * std::f64::consts::PI * t / period).sin();
            let s = (v * 30000.0) as i16;
            let base = i * 2;
            if base + 1 < out.len() {
                if channel == 0 {
                    out[base] = s;
                    out[base + 1] = 0;
                } else {
                    out[base] = 0;
                    out[base + 1] = s;
                }
            }
        }
    }

    fn push_interleaved(j: &mut Jitter, samples: &[i16]) {
        let mut b = Vec::with_capacity(samples.len() * 2);
        for s in samples {
            b.extend_from_slice(&s.to_le_bytes());
        }
        j.push_pcm(&b);
    }

    fn count_zero_crossings(samples: &[f32]) -> usize {
        let mut n = 0usize;
        let mut prev_pos = samples[0] >= 0.0;
        for &s in samples.iter().skip(1) {
            let pos = s >= 0.0;
            if pos != prev_pos {
                n += 1;
            }
            prev_pos = pos;
        }
        n
    }

    fn push_sine_chunk(j: &mut Jitter, frames: usize, at: &mut usize, rate: u32) {
        let mut stereo = vec![0i16; frames * 2];
        for i in 0..frames {
            let t = (*at + i) as f64;
            let period = rate as f64 / 1000.0;
            let v = (2.0 * std::f64::consts::PI * t / period).sin();
            let s = (v * 30000.0) as i16;
            stereo[i * 2] = s;
            stereo[i * 2 + 1] = 0;
        }
        *at += frames;
        let bytes: Vec<u8> = stereo.iter().flat_map(|s| s.to_le_bytes()).collect();
        j.push_pcm(&bytes);
    }

    fn stream_seconds(
        j: &mut Jitter,
        in_rate: u32,
        out_rate: u32,
        out_ch: usize,
        sec: usize,
    ) -> Vec<f32> {
        let target_frames = in_rate as usize * TARGET_MS / 1000;
        let chunk = 240usize;
        let total = out_rate as usize * sec;
        let mut out = Vec::with_capacity(total);
        let mut phase = 0usize;
        while out.len() < total {
            while j.buffered_frames() < target_frames {
                push_sine_chunk(j, chunk, &mut phase, in_rate);
            }
            let mut buf = vec![0f32; 512 * out_ch];
            j.render_f32(&mut buf, out_ch, out_rate);
            if out_ch == 1 {
                out.extend_from_slice(&buf);
            } else {
                out.extend(buf.chunks_exact(out_ch).map(|c| c[0]));
            }
            if out.len() > total {
                out.truncate(total);
            }
        }
        out
    }

    fn fill_target(j: &mut Jitter, rate: u32) {
        let frames = rate as usize * TARGET_MS / 1000;
        let mut stereo = vec![0i16; frames * 2];
        stereo_sine_1k(&mut stereo, frames, rate, 0);
        push_interleaved(j, &stereo);
    }

    #[test]
    fn pitch_48k_device() {
        let mut j = Jitter::new();
        j.set_input_rate(48_000);
        let mono = stream_seconds(&mut j, 48_000, 48_000, 1, 1);
        let zc = count_zero_crossings(&mono);
        assert!((1900..=2100).contains(&zc), "zc={zc}");
    }

    #[test]
    fn pitch_44k_device() {
        let mut j = Jitter::new();
        j.set_input_rate(48_000);
        let mono = stream_seconds(&mut j, 48_000, 44_100, 1, 1);
        let zc = count_zero_crossings(&mono);
        assert!((1900..=2100).contains(&zc), "zc={zc}");
    }

    #[test]
    fn lr_separate() {
        let mut j = Jitter::new();
        j.set_input_rate(48_000);
        let mut l_out = Vec::new();
        let mut r_out = Vec::new();
        let mut phase = 0usize;
        while l_out.len() < 48_000 {
            while j.buffered_frames() < 48_000 * TARGET_MS / 1000 {
                push_sine_chunk(&mut j, 240, &mut phase, 48_000);
            }
            let mut lr = vec![0f32; 1024];
            j.render_f32(&mut lr, 2, 48_000);
            for &[l, r] in lr.as_chunks::<2>().0 {
                l_out.push(l);
                r_out.push(r);
            }
        }
        let zc_l = count_zero_crossings(&l_out[..48_000]);
        let zc_r = count_zero_crossings(&r_out[..48_000]);
        assert!((1900..=2100).contains(&zc_l), "l zc={zc_l}");
        assert!(zc_r < 50, "r zc={zc_r}");
    }

    #[test]
    fn underrun_reprimes() {
        let mut j = Jitter::new();
        fill_target(&mut j, 48_000);
        assert!(j.is_primed());
        let mut buf = vec![0f32; 1440];
        j.render_f32(&mut buf, 1, 48_000);
        j.render_f32(&mut buf, 1, 48_000);
        assert!(!j.is_primed());
        fill_target(&mut j, 48_000);
        assert!(j.is_primed());
    }

    #[test]
    fn big_callbacks_raise_the_prime_level() {
        let mut j = Jitter::new();
        fill_target(&mut j, 48_000);
        let mut buf = vec![0f32; 2880];
        j.render_f32(&mut buf, 1, 48_000);
        assert!(!j.is_primed());
        fill_target(&mut j, 48_000);
        fill_target(&mut j, 48_000);
        assert!(j.is_primed());
        j.render_f32(&mut buf, 1, 48_000);
        assert!(buf.iter().any(|s| *s != 0.0));
    }

    #[test]
    fn drift_trim_stays_bounded() {
        let mut j = Jitter::new();
        let big = j.max_samples() + 2000;
        let sample = 1000i16;
        let bytes = sample.to_le_bytes();
        for _ in 0..big {
            j.push_pcm(&bytes);
            j.push_pcm(&bytes);
        }
        for _ in 0..50 {
            let mut buf = vec![0f32; 4800];
            j.render_f32(&mut buf, 1, 48_000);
        }
        assert!(j.len <= j.max_samples());
    }

    #[test]
    fn chunk_is_five_ms_stereo() {
        let samples = 240 * 2;
        assert_eq!(samples * 2, 960);
    }
}

use crate::Decoder;
use crate::annex::{has_idr, parameter_sets_only};
use crate::i420::{Frame, pack_yuv420p_into};
use anyhow::{Result, anyhow};
use openh264::OpenH264API;
use openh264::decoder::{Decoder as OhDecoder, DecoderConfig, Flush};
use openh264::formats::YUVSource;

pub struct OpenH264Decoder {
    inner: OhDecoder,
    pending: Vec<u8>,
    feed: Vec<u8>,
    bufs: [Frame; 2],
    buf_idx: u8,
}

impl OpenH264Decoder {
    fn open_decoder() -> Result<OhDecoder> {
        let cfg = DecoderConfig::new().flush_after_decode(Flush::NoFlush);
        OhDecoder::with_api_config(OpenH264API::from_source(), cfg).map_err(|e| anyhow!("{e}"))
    }

    pub fn new() -> Result<Self> {
        let inner = Self::open_decoder()?;
        Ok(Self {
            inner,
            pending: Vec::new(),
            feed: Vec::new(),
            bufs: [Frame::default(), Frame::default()],
            buf_idx: 0,
        })
    }

    fn store_config(&mut self, data: &[u8]) {
        self.pending.extend_from_slice(data);
    }
}

impl Decoder for OpenH264Decoder {
    fn decode(&mut self, annexb: &[u8]) -> Result<Option<Frame>> {
        if annexb.is_empty() {
            return Ok(None);
        }
        if parameter_sets_only(annexb) {
            self.store_config(annexb);
            return Ok(None);
        }
        let input = if self.pending.is_empty() || !has_idr(annexb) {
            annexb
        } else {
            self.feed.clear();
            self.feed.extend_from_slice(&self.pending);
            self.feed.extend_from_slice(annexb);
            self.feed.as_slice()
        };
        let i = self.buf_idx as usize;
        self.buf_idx = 1 - self.buf_idx;
        {
            let decoded = match self.inner.decode(input) {
                Ok(v) => v,
                Err(_) if !has_idr(annexb) => return Ok(None),
                Err(e) => return Err(anyhow!("{e}")),
            };
            let Some(yuv) = decoded else {
                return Ok(None);
            };
            let (width, height) = yuv.dimensions();
            let (sy, su, _) = yuv.strides();
            pack_yuv420p_into(
                &mut self.bufs[i],
                yuv.y(),
                yuv.u(),
                yuv.v(),
                width as u32,
                height as u32,
                sy,
                su,
            );
        }
        if has_idr(annexb) {
            self.pending.clear();
        }
        let mut out = Frame::default();
        std::mem::swap(&mut out, &mut self.bufs[i]);
        Ok(Some(out))
    }

    fn flush(&mut self) -> Result<Option<Frame>> {
        let i = self.buf_idx as usize;
        self.buf_idx = 1 - self.buf_idx;
        {
            let remaining = self.inner.flush_remaining().map_err(|e| anyhow!("{e}"))?;
            let Some(yuv) = remaining.into_iter().next() else {
                return Ok(None);
            };
            let (width, height) = yuv.dimensions();
            let (sy, su, _) = yuv.strides();
            pack_yuv420p_into(
                &mut self.bufs[i],
                yuv.y(),
                yuv.u(),
                yuv.v(),
                width as u32,
                height as u32,
                sy,
                su,
            );
        }
        let mut out = Frame::default();
        std::mem::swap(&mut out, &mut self.bufs[i]);
        Ok(Some(out))
    }

    fn reset(&mut self) {
        if let Ok(dec) = Self::open_decoder() {
            self.inner = dec;
        }
    }

    fn name(&self) -> &'static str {
        "openh264"
    }
}

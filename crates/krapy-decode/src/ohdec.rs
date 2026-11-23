use crate::Decoder;
use crate::annex::nal_starts;
use crate::i420::{Frame, pack_yuv420p_into};
use anyhow::{Result, anyhow};
use openh264::OpenH264API;
use openh264::decoder::{Decoder as OhDecoder, DecoderConfig, Flush};
use openh264::formats::YUVSource;

pub struct OpenH264Decoder {
    inner: OhDecoder,
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
            bufs: [Frame::default(), Frame::default()],
            buf_idx: 0,
        })
    }
}

impl Decoder for OpenH264Decoder {
    fn decode(&mut self, annexb: &[u8]) -> Result<Option<Frame>> {
        if annexb.is_empty() {
            return Ok(None);
        }
        let input = annexb;
        let i = self.buf_idx as usize;
        self.buf_idx = 1 - self.buf_idx;
        {
            let decoded = match self.inner.decode(input) {
                Ok(v) => v,
                // openh264 reports errors on some fine p-frames
                Err(_) if !nal_starts(annexb).iter().any(|&(_, t)| t == 5) => return Ok(None),
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

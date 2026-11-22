use crate::Decoder;
use crate::i420::{Frame, pack_yuv420p};
use anyhow::{Result, anyhow};
use openh264::decoder::Decoder as OhDecoder;
use openh264::formats::YUVSource;

pub struct OpenH264Decoder {
    inner: OhDecoder,
}

impl OpenH264Decoder {
    pub fn new() -> Result<Self> {
        let inner = OhDecoder::new().map_err(|e| anyhow!("{e}"))?;
        Ok(Self { inner })
    }

    fn yuv_to_frame(yuv: &impl YUVSource) -> Frame {
        let (width, height) = yuv.dimensions();
        let (sy, su, _sv) = yuv.strides();
        pack_yuv420p(
            yuv.y(),
            yuv.u(),
            yuv.v(),
            width as u32,
            height as u32,
            sy,
            su,
        )
    }
}

impl Decoder for OpenH264Decoder {
    fn decode(&mut self, annexb: &[u8]) -> Result<Option<Frame>> {
        if annexb.is_empty() {
            return Ok(None);
        }
        match self.inner.decode(annexb).map_err(|e| anyhow!("{e}"))? {
            Some(yuv) => Ok(Some(Self::yuv_to_frame(&yuv))),
            None => Ok(None),
        }
    }

    fn flush(&mut self) -> Result<Option<Frame>> {
        let remaining = self.inner.flush_remaining().map_err(|e| anyhow!("{e}"))?;
        Ok(remaining.first().map(Self::yuv_to_frame))
    }

    fn reset(&mut self) {
        if let Ok(dec) = Self::new() {
            self.inner = dec.inner;
        }
    }

    fn name(&self) -> &'static str {
        "openh264"
    }
}

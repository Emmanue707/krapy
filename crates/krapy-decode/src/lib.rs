mod i420;

#[cfg(feature = "ffmpeg")]
mod ffdec;

#[cfg(feature = "openh264")]
mod ohdec;

pub use i420::Frame;

pub trait Decoder: Send {
    fn decode(&mut self, annexb: &[u8]) -> anyhow::Result<Option<Frame>>;
    fn flush(&mut self) -> anyhow::Result<Option<Frame>> {
        Ok(None)
    }
    fn reset(&mut self);
    fn name(&self) -> &'static str;
}

pub fn available() -> Vec<&'static str> {
    let mut v = Vec::new();
    if cfg!(feature = "ffmpeg") {
        v.push("ffmpeg");
    }
    if cfg!(feature = "openh264") {
        v.push("openh264");
    }
    v
}

pub fn open(preferred: Option<&str>) -> anyhow::Result<Box<dyn Decoder>> {
    let mut order = available();
    if let Some(want) = preferred {
        order.retain(|name| *name != want);
        order.insert(0, want);
    }
    let mut last = None;
    for name in order {
        match try_open(name) {
            Ok(dec) => return Ok(dec),
            Err(e) => last = Some(e),
        }
    }
    Err(last.unwrap_or_else(|| anyhow::anyhow!("no decoder backend")))
}

fn try_open(name: &str) -> anyhow::Result<Box<dyn Decoder>> {
    match name {
        #[cfg(feature = "ffmpeg")]
        "ffmpeg" => Ok(Box::new(ffdec::FfmpegDecoder::new()?)),
        #[cfg(feature = "openh264")]
        "openh264" => Ok(Box::new(ohdec::OpenH264Decoder::new()?)),
        other => Err(anyhow::anyhow!("decoder backend not available: {other}")),
    }
}
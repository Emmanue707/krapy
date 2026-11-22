use crate::Decoder;
use crate::i420::{Frame, pack_yuv420p};
use anyhow::{Result, anyhow};
use ffmpeg_next::Error as FfmpegError;
use ffmpeg_next::codec::context::Context;
use ffmpeg_next::codec::flag::Flags;
use ffmpeg_next::codec::id::Id;
use ffmpeg_next::codec::threading;
use ffmpeg_next::packet::Packet;
use ffmpeg_next::software::scaling::{context::Context as Scaler, flag::Flags as ScaleFlags};
use ffmpeg_next::util::format::pixel::Pixel;
use ffmpeg_next::util::frame::video::Video as VideoFrame;
use std::sync::Once;

static INIT: Once = Once::new();

pub struct FfmpegDecoder {
    decoder: ffmpeg_next::codec::decoder::Video,
}

impl FfmpegDecoder {
    pub fn new() -> Result<Self> {
        INIT.call_once(|| {
            let _ = ffmpeg_next::init();
        });
        let codec = ffmpeg_next::codec::decoder::find(Id::H264)
            .ok_or_else(|| anyhow!("h264 decoder not found"))?;
        let mut ctx = Context::new_with_codec(codec);
        ctx.set_flags(Flags::LOW_DELAY);
        ctx.set_threading(threading::Config {
            kind: threading::Type::Slice,
            count: 1,
        });
        let decoder = ctx.decoder().video().map_err(ffmpeg_err)?;
        Ok(Self { decoder })
    }

    fn try_receive(&mut self) -> Result<Option<Frame>> {
        let mut frame = VideoFrame::empty();
        match self.decoder.receive_frame(&mut frame) {
            Ok(()) => Ok(Some(video_to_frame(&frame)?)),
            Err(e) if is_eagain(&e) => Ok(None),
            Err(e) => Err(ffmpeg_err(e)),
        }
    }
}

fn video_to_frame(frame: &VideoFrame) -> Result<Frame> {
    let width = frame.width();
    let height = frame.height();
    let format = frame.format();
    if format == Pixel::YUV420P {
        return Ok(pack_yuv420p(
            frame.data(0),
            frame.data(1),
            frame.data(2),
            width,
            height,
            frame.stride(0),
            frame.stride(1),
        ));
    }
    let mut scaler = Scaler::get(
        format,
        width,
        height,
        Pixel::YUV420P,
        width,
        height,
        ScaleFlags::BILINEAR,
    )
    .map_err(ffmpeg_err)?;
    let mut out = VideoFrame::empty();
    scaler.run(frame, &mut out).map_err(ffmpeg_err)?;
    Ok(pack_yuv420p(
        out.data(0),
        out.data(1),
        out.data(2),
        width,
        height,
        out.stride(0),
        out.stride(1),
    ))
}

impl Decoder for FfmpegDecoder {
    fn decode(&mut self, annexb: &[u8]) -> Result<Option<Frame>> {
        if annexb.is_empty() {
            return Ok(None);
        }
        let pkt = Packet::copy(annexb);
        self.decoder.send_packet(&pkt).map_err(ffmpeg_err)?;
        self.try_receive()
    }

    fn flush(&mut self) -> Result<Option<Frame>> {
        self.decoder.send_eof().map_err(ffmpeg_err)?;
        self.try_receive()
    }

    fn reset(&mut self) {
        self.decoder.flush();
    }

    fn name(&self) -> &'static str {
        "ffmpeg"
    }
}

fn ffmpeg_err(e: FfmpegError) -> anyhow::Error {
    anyhow!("{e}")
}

fn is_eagain(e: &FfmpegError) -> bool {
    matches!(e, FfmpegError::Other { errno } if *errno == libc::EAGAIN)
}

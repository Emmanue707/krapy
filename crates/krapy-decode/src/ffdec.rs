use crate::Decoder;
use crate::color::{ColorSpec, Matrix};
use crate::i420::{Frame, pack_yuv420p_into};
use anyhow::{Result, anyhow};
use ffmpeg_next::Error as FfmpegError;
use ffmpeg_next::codec::context::Context;
use ffmpeg_next::codec::flag::Flags;
use ffmpeg_next::codec::id::Id;
use ffmpeg_next::codec::threading;
use ffmpeg_next::packet::Packet;
use ffmpeg_next::software::scaling::{context::Context as Scaler, flag::Flags as ScaleFlags};
use ffmpeg_next::util::color::{Range, Space};
use ffmpeg_next::util::format::pixel::Pixel;
use ffmpeg_next::util::frame::video::Video as VideoFrame;
use std::sync::Once;
use std::sync::atomic::{AtomicBool, Ordering};

static INIT: Once = Once::new();
static VERBOSE: AtomicBool = AtomicBool::new(false);

pub fn set_log_verbose(verbose: bool) {
    VERBOSE.store(verbose, Ordering::Relaxed);
    let level = if verbose {
        ffmpeg_next::util::log::Level::Warning
    } else {
        ffmpeg_next::util::log::Level::Quiet
    };
    ffmpeg_next::util::log::set_level(level);
}

pub struct FfmpegDecoder {
    decoder: ffmpeg_next::codec::decoder::Video,
    video: VideoFrame,
    bufs: [Frame; 2],
    buf_idx: u8,
}

impl FfmpegDecoder {
    pub fn new() -> Result<Self> {
        INIT.call_once(|| {
            let _ = ffmpeg_next::init();
            if !VERBOSE.load(Ordering::Relaxed) {
                ffmpeg_next::util::log::set_level(ffmpeg_next::util::log::Level::Quiet);
            }
        });
        let codec = ffmpeg_next::codec::decoder::find(Id::H264)
            .ok_or_else(|| anyhow!("h264 decoder not found"))?;
        let mut ctx = Context::new_with_codec(codec);
        ctx.set_flags(Flags::LOW_DELAY);
        // ffmpeg-sys types this differently depending on how it was built
        #[allow(clippy::unnecessary_cast)]
        unsafe {
            (*ctx.as_mut_ptr()).flags2 |= ffmpeg_next::ffi::AV_CODEC_FLAG2_FAST as i32;
        }
        ctx.set_threading(threading::Config {
            kind: threading::Type::Slice,
            count: 1,
        });
        let decoder = ctx.decoder().video().map_err(ffmpeg_err)?;
        Ok(Self {
            decoder,
            video: VideoFrame::empty(),
            bufs: [Frame::default(), Frame::default()],
            buf_idx: 0,
        })
    }

    fn try_receive(&mut self) -> Result<Option<Frame>> {
        match self.decoder.receive_frame(&mut self.video) {
            Ok(()) => Ok(Some(self.video_to_frame()?)),
            Err(e) if is_eagain(&e) => Ok(None),
            Err(e) => Err(ffmpeg_err(e)),
        }
    }

    fn video_to_frame(&mut self) -> Result<Frame> {
        let frame = &self.video;
        let width = frame.width();
        let height = frame.height();
        let format = frame.format();
        let i = self.buf_idx as usize;
        self.buf_idx = 1 - self.buf_idx;
        let out = &mut self.bufs[i];
        if format == Pixel::YUV420P {
            pack_yuv420p_into(
                out,
                frame.data(0),
                frame.data(1),
                frame.data(2),
                width,
                height,
                frame.stride(0),
                frame.stride(1),
            );
        } else {
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
            let mut yuv = VideoFrame::empty();
            scaler.run(frame, &mut yuv).map_err(ffmpeg_err)?;
            pack_yuv420p_into(
                out,
                yuv.data(0),
                yuv.data(1),
                yuv.data(2),
                width,
                height,
                yuv.stride(0),
                yuv.stride(1),
            );
        }
        self.bufs[i].color = color_from_video(frame);
        let mut taken = Frame::default();
        std::mem::swap(&mut taken, &mut self.bufs[i]);
        Ok(taken)
    }
}

fn color_from_video(frame: &VideoFrame) -> ColorSpec {
    let full_range = frame.color_range() == Range::JPEG;
    let matrix = match frame.color_space() {
        Space::SMPTE170M | Space::BT470BG | Space::FCC => Matrix::Bt601,
        Space::BT709 | Space::Unspecified => Matrix::Bt709,
        _ => Matrix::Bt709,
    };
    ColorSpec { full_range, matrix }
}

impl Decoder for FfmpegDecoder {
    fn decode(&mut self, annexb: &[u8]) -> Result<Option<Frame>> {
        if annexb.is_empty() {
            return Ok(None);
        }
        {
            let pkt = Packet::borrow(annexb);
            self.decoder.send_packet(&pkt).map_err(ffmpeg_err)?;
        }
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

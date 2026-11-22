use crate::Decoder;
use crate::annex::{has_idr, parameter_sets_only};
use crate::i420::{Frame, pack_yuv420p_into};
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
    video: VideoFrame,
    bufs: [Frame; 2],
    buf_idx: u8,
    pending: Vec<u8>,
    feed: Vec<u8>,
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
        Ok(Self {
            decoder,
            video: VideoFrame::empty(),
            bufs: [Frame::default(), Frame::default()],
            buf_idx: 0,
            pending: Vec::new(),
            feed: Vec::new(),
        })
    }

    fn store_config(&mut self, data: &[u8]) {
        self.pending.extend_from_slice(data);
        let pkt = Packet::borrow(data);
        let _ = self.decoder.send_packet(&pkt);
        let _ = self.try_receive();
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
        let mut taken = Frame::default();
        std::mem::swap(&mut taken, &mut self.bufs[i]);
        Ok(taken)
    }
}

impl Decoder for FfmpegDecoder {
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
        {
            let pkt = Packet::borrow(input);
            self.decoder.send_packet(&pkt).map_err(ffmpeg_err)?;
        }
        let frame = self.try_receive()?;
        if frame.is_some() && has_idr(annexb) {
            self.pending.clear();
        }
        Ok(frame)
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

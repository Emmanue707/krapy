use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, Stream};
use krapy_transport::AudioQueue;
use parking_lot::Mutex;

use crate::jitter::Jitter;

pub struct AudioOutput {
    enabled: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    jitter: Arc<Mutex<Jitter>>,
    feed: JoinHandle<()>,
    _stream: Stream,
}

impl AudioOutput {
    pub fn from_queue(
        queue: Arc<AudioQueue>,
        enabled: Arc<AtomicBool>,
        stop: Arc<AtomicBool>,
    ) -> Option<Self> {
        let host = cpal::default_host();
        let device = host.default_output_device()?;
        let config = device.default_output_config().ok()?;
        let jitter = Arc::new(Mutex::new(Jitter::new()));
        let stream = build_stream(&device, &config, Arc::clone(&jitter), Arc::clone(&enabled))?;
        stream.play().ok()?;
        let feed = spawn_feed(queue, Arc::clone(&jitter), Arc::clone(&stop));
        Some(Self {
            enabled,
            stop,
            jitter,
            feed,
            _stream: stream,
        })
    }

    pub fn set_enabled(&self, on: bool) {
        self.enabled.store(on, Ordering::Relaxed);
        if !on {
            let mut scratch = [0i16; 4096];
            self.jitter.lock().pop_into(&mut scratch);
        }
    }

    pub fn stop(self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = self.feed.join();
    }
}

fn spawn_feed(
    queue: Arc<AudioQueue>,
    jitter: Arc<Mutex<Jitter>>,
    stop: Arc<AtomicBool>,
) -> JoinHandle<()> {
    thread::Builder::new()
        .name("krapy-audio-feed".into())
        .spawn(move || {
            let mut buf = [0u8; 960];
            while !stop.load(Ordering::Relaxed) {
                if let Some((_, _, n)) = queue.pop(&mut buf) {
                    jitter.lock().push_pcm(&buf[..n]);
                } else {
                    thread::sleep(Duration::from_millis(2));
                }
            }
        })
        .expect("audio feed")
}

fn build_stream(
    device: &cpal::Device,
    config: &cpal::SupportedStreamConfig,
    jitter: Arc<Mutex<Jitter>>,
    enabled: Arc<AtomicBool>,
) -> Option<Stream> {
    let channels = config.channels() as usize;
    let sample_rate = config.sample_rate().0;
    let err = |e| tracing::warn!("audio stream: {e}");
    let stream_cfg: cpal::StreamConfig = config.clone().into();
    match config.sample_format() {
        SampleFormat::I16 => device
            .build_output_stream(
                &stream_cfg,
                move |data: &mut [i16], _| fill_i16(data, channels, sample_rate, &jitter, &enabled),
                err,
                None,
            )
            .ok(),
        SampleFormat::F32 => device
            .build_output_stream(
                &stream_cfg,
                move |data: &mut [f32], _| fill_f32(data, channels, sample_rate, &jitter, &enabled),
                err,
                None,
            )
            .ok(),
        _ => {
            tracing::warn!("unsupported audio sample format");
            None
        }
    }
}

fn fill_i16(
    data: &mut [i16],
    channels: usize,
    sample_rate: u32,
    jitter: &Mutex<Jitter>,
    enabled: &AtomicBool,
) {
    if !enabled.load(Ordering::Relaxed) {
        data.fill(0);
        return;
    }
    let frames = data.len() / channels;
    let mut mono = [0i16; 2048];
    let take = frames.min(mono.len());
    let got = {
        let mut j = jitter.lock();
        j.pop_into(&mut mono[..take])
    };
    for (i, frame) in data.chunks_mut(channels).enumerate() {
        let idx = resample_idx(i, sample_rate);
        let s = if idx < got { mono[idx] } else { 0 };
        frame.fill(s);
    }
}

fn fill_f32(
    data: &mut [f32],
    channels: usize,
    sample_rate: u32,
    jitter: &Mutex<Jitter>,
    enabled: &AtomicBool,
) {
    if !enabled.load(Ordering::Relaxed) {
        data.fill(0.0);
        return;
    }
    let frames = data.len() / channels;
    let mut mono = [0i16; 2048];
    let take = frames.min(mono.len());
    let got = {
        let mut j = jitter.lock();
        j.pop_into(&mut mono[..take])
    };
    for (i, frame) in data.chunks_mut(channels).enumerate() {
        let idx = resample_idx(i, sample_rate);
        let s = if idx < got {
            mono[idx] as f32 / 32768.0
        } else {
            0.0
        };
        frame.fill(s);
    }
}

fn resample_idx(out_frame: usize, sample_rate: u32) -> usize {
    if sample_rate == 48_000 {
        out_frame
    } else {
        out_frame * 48_000 / sample_rate as usize
    }
}

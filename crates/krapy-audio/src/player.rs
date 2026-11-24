use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, Stream, SupportedStreamConfigRange};
use krapy_transport::AudioQueue;
use parking_lot::Mutex;

use crate::jitter::Jitter;

pub struct AudioOutput {
    enabled: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    input_rate: Arc<AtomicU32>,
    jitter: Arc<Mutex<Jitter>>,
    feed: JoinHandle<()>,
    _stream: Stream,
}

impl AudioOutput {
    pub fn from_queue(
        queue: Arc<AudioQueue>,
        enabled: Arc<AtomicBool>,
        stop: Arc<AtomicBool>,
        input_rate: Arc<AtomicU32>,
    ) -> Option<Self> {
        let host = cpal::default_host();
        let device = host.default_output_device()?;
        let config = pick_config(&device)?;
        let jitter = Arc::new(Mutex::new(Jitter::new()));
        jitter
            .lock()
            .set_input_rate(input_rate.load(Ordering::Relaxed));
        let stream = build_stream(
            &device,
            &config,
            Arc::clone(&jitter),
            Arc::clone(&enabled),
            Arc::clone(&input_rate),
        )?;
        stream.play().ok()?;
        let feed = spawn_feed(queue, Arc::clone(&jitter), Arc::clone(&stop));
        Some(Self {
            enabled,
            stop,
            input_rate,
            jitter,
            feed,
            _stream: stream,
        })
    }

    pub fn set_enabled(&self, on: bool) {
        self.enabled.store(on, Ordering::Relaxed);
        if !on {
            self.jitter.lock().clear();
        }
    }

    pub fn set_input_rate(&self, rate: u32) {
        self.input_rate.store(rate, Ordering::Relaxed);
        self.jitter.lock().set_input_rate(rate);
    }

    pub fn stop(self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = self.feed.join();
    }
}

fn pick_config(device: &cpal::Device) -> Option<cpal::SupportedStreamConfig> {
    let mut i16_pick: Option<cpal::SupportedStreamConfig> = None;
    if let Ok(configs) = device.supported_output_configs() {
        for range in configs {
            if !supports_48k(&range) {
                continue;
            }
            let with_rate = range.with_sample_rate(cpal::SampleRate(48_000));
            match range.sample_format() {
                SampleFormat::F32 => return Some(with_rate),
                SampleFormat::I16 if i16_pick.is_none() => i16_pick = Some(with_rate),
                _ => {}
            }
        }
    }
    if let Some(c) = i16_pick {
        return Some(c);
    }
    device.default_output_config().ok()
}

fn supports_48k(range: &SupportedStreamConfigRange) -> bool {
    range.min_sample_rate().0 <= 48_000 && range.max_sample_rate().0 >= 48_000
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
                if let Some((_, _, n)) = queue.pop_wait(Duration::from_millis(1), &mut buf) {
                    jitter.lock().push_pcm(&buf[..n]);
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
    input_rate: Arc<AtomicU32>,
) -> Option<Stream> {
    let channels = config.channels() as usize;
    let sample_rate = config.sample_rate().0;
    let err = |e| tracing::warn!("audio stream: {e}");
    let mut stream_cfg: cpal::StreamConfig = config.clone().into();
    if let cpal::SupportedBufferSize::Range { min, max } = config.buffer_size() {
        let want = sample_rate / 100;
        if *min <= want && want <= *max {
            stream_cfg.buffer_size = cpal::BufferSize::Fixed(want);
        }
    }
    match config.sample_format() {
        SampleFormat::I16 => device
            .build_output_stream(
                &stream_cfg,
                move |data: &mut [i16], _| {
                    run_i16(data, channels, sample_rate, &jitter, &enabled, &input_rate)
                },
                err,
                None,
            )
            .ok(),
        SampleFormat::F32 => device
            .build_output_stream(
                &stream_cfg,
                move |data: &mut [f32], _| {
                    run_f32(data, channels, sample_rate, &jitter, &enabled, &input_rate)
                },
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

fn run_i16(
    data: &mut [i16],
    channels: usize,
    sample_rate: u32,
    jitter: &Mutex<Jitter>,
    enabled: &AtomicBool,
    input_rate: &AtomicU32,
) {
    if !enabled.load(Ordering::Relaxed) {
        data.fill(0);
        return;
    }
    {
        let mut j = jitter.lock();
        let rate = input_rate.load(Ordering::Relaxed);
        if rate != j.input_rate() {
            j.set_input_rate(rate);
        }
        j.render_i16(data, channels, sample_rate);
    }
}

fn run_f32(
    data: &mut [f32],
    channels: usize,
    sample_rate: u32,
    jitter: &Mutex<Jitter>,
    enabled: &AtomicBool,
    input_rate: &AtomicU32,
) {
    if !enabled.load(Ordering::Relaxed) {
        data.fill(0.0);
        return;
    }
    {
        let mut j = jitter.lock();
        let rate = input_rate.load(Ordering::Relaxed);
        if rate != j.input_rate() {
            j.set_input_rate(rate);
        }
        j.render_f32(data, channels, sample_rate);
    }
}

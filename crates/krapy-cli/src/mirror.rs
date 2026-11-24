use std::sync::Once;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use krapy_audio::AudioOutput;
use krapy_core::adaptive::Adaptive;
use krapy_core::device::{PairedDevice, Store};
use krapy_core::preset::{Preset, Quality};
use krapy_decode::{Decoder, open};
use krapy_render::{FrameSource, MenuStats, PhoneSettings};
use krapy_transport::{AcceptMode, Control, FrameQueue, Listener, Session, SessionError};

const PAIR_WINDOW: Duration = Duration::from_secs(300);
const POLL: Duration = Duration::from_millis(50);
const NET_TICK: Duration = Duration::from_millis(20);
const KEY_RETRY: Duration = Duration::from_millis(300);
const SMALL_STACK: usize = 256 * 1024;

type ControlSlot = Arc<Mutex<Option<Sender<Control>>>>;

#[derive(Clone, Debug)]
pub struct MirrorOpts {
    pub preset: Preset,
    pub decoder: Option<String>,
    pub max_size: Option<u16>,
    pub bitrate: Option<u32>,
    pub fps: Option<u8>,
    pub stats: bool,
    pub no_window: bool,
    pub verbose: bool,
    pub no_audio: bool,
    pub show_touches: bool,
}

impl MirrorOpts {
    pub fn ceiling(&self) -> Quality {
        let mut q = self.preset.quality();
        if let Some(m) = self.max_size {
            q.max_size = m;
        }
        if let Some(b) = self.bitrate {
            q.bitrate = b;
        }
        if let Some(f) = self.fps {
            q.fps = f;
        }
        q
    }
}

pub fn desktop_id_bytes(id_hex: &str) -> Result<[u8; 8]> {
    let bytes = hex::decode(id_hex).context("desktop id hex")?;
    if bytes.len() != 8 {
        anyhow::bail!("desktop id must be 8 bytes");
    }
    let mut out = [0u8; 8];
    out.copy_from_slice(&bytes);
    Ok(out)
}

pub fn lookup_secrets(store: &Store) -> Vec<(String, [u8; 32])> {
    store
        .devices()
        .iter()
        .filter_map(|d| {
            let bytes = hex::decode(&d.secret).ok()?;
            if bytes.len() != 32 {
                return None;
            }
            let mut secret = [0u8; 32];
            secret.copy_from_slice(&bytes);
            Some((d.id.clone(), secret))
        })
        .collect()
}

struct Counters {
    frames_in: AtomicU64,
    frames_out: AtomicU64,
    decode_us: AtomicU64,
    decode_samples: AtomicU64,
    bitrate: AtomicU64,
    lat_us: AtomicU64,
    lat_samples: AtomicU64,
    lat_max_us: AtomicU64,
    total_us: AtomicU64,
    total_samples: AtomicU64,
    total_max_us: AtomicU64,
    net_us: AtomicU64,
    pipe_decode_us: AtomicU64,
}

impl Counters {
    fn new() -> Self {
        Self {
            frames_in: AtomicU64::new(0),
            frames_out: AtomicU64::new(0),
            decode_us: AtomicU64::new(0),
            decode_samples: AtomicU64::new(0),
            bitrate: AtomicU64::new(0),
            lat_us: AtomicU64::new(0),
            lat_samples: AtomicU64::new(0),
            lat_max_us: AtomicU64::new(0),
            total_us: AtomicU64::new(0),
            total_samples: AtomicU64::new(0),
            total_max_us: AtomicU64::new(0),
            net_us: AtomicU64::new(0),
            pipe_decode_us: AtomicU64::new(0),
        }
    }
}

fn preset_from_id(id: u8) -> Preset {
    match id {
        0 => Preset::LowLatency,
        2 => Preset::HighQuality,
        _ => Preset::Balanced,
    }
}

pub fn run_connect_loop(
    listener: Listener,
    store: Store,
    source: FrameSource,
    control_slot: ControlSlot,
    opts: MirrorOpts,
) -> Result<()> {
    let map = lookup_secrets(&store);
    loop {
        let session = match accept_known(&listener, &map) {
            Ok(s) => s,
            Err(SessionError::HandshakeTimeout) => continue,
            Err(e) => return Err(anyhow::anyhow!("{e:?}")),
        };
        if let Ok(mut g) = control_slot.lock() {
            *g = Some(session.control_tx.clone());
        }
        let why = drive_session(session, source.clone(), &opts)?;
        if let Ok(mut g) = control_slot.lock() {
            *g = None;
        }
        println!("session ended ({why}), waiting for the phone to reconnect");
    }
}

pub fn run_pair_then_connect(
    listener: Listener,
    mut store: Store,
    source: FrameSource,
    control_slot: ControlSlot,
    k: [u8; 32],
    opts: MirrorOpts,
    phone_connected: Option<Arc<AtomicBool>>,
) -> Result<()> {
    let deadline = Instant::now() + PAIR_WINDOW;
    let secret_cell = Arc::new(Mutex::new(None::<[u8; 32]>));
    let session = loop {
        if Instant::now() >= deadline {
            println!("pair window expired (5 minutes)");
            return Ok(());
        }
        let cell = Arc::clone(&secret_cell);
        let mut cb = move |_pid: [u8; 8], s: [u8; 32]| {
            if let Ok(mut g) = cell.lock() {
                *g = Some(s);
            }
        };
        match listener.accept(AcceptMode::Pair {
            k,
            on_paired: &mut cb,
        }) {
            Ok(s) => {
                if let Some(flag) = &phone_connected {
                    flag.store(true, Ordering::Relaxed);
                }
                break s;
            }
            Err(SessionError::HandshakeTimeout) => continue,
            Err(e) => return Err(anyhow::anyhow!("{e:?}")),
        }
    };
    let secret = secret_cell
        .lock()
        .ok()
        .and_then(|g| *g)
        .context("pair secret")?;
    store.upsert(PairedDevice {
        id: hex::encode(session.phone_id()),
        name: session.phone_name().to_string(),
        alias: None,
        secret: hex::encode(secret),
        last_addr: None,
        last_seen: Some(now_secs()),
    })?;
    if let Ok(mut g) = control_slot.lock() {
        *g = Some(session.control_tx.clone());
    }
    let name = session.phone_name().to_string();
    let why = drive_session(session, source.clone(), &opts)?;
    if let Ok(mut g) = control_slot.lock() {
        *g = None;
    }
    println!("paired with {name}; session ended ({why}), waiting for the phone to reconnect");
    run_connect_loop(listener, store, source, control_slot, opts)
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn accept_known(listener: &Listener, map: &[(String, [u8; 32])]) -> Result<Session, SessionError> {
    listener.accept(AcceptMode::Known {
        lookup: &|phone_id: [u8; 8]| {
            let id = hex::encode(phone_id);
            map.iter().find(|(k, _)| k == &id).map(|(_, s)| *s)
        },
    })
}

fn drive_session(
    mut session: Session,
    source: FrameSource,
    opts: &MirrorOpts,
) -> Result<&'static str> {
    let ceiling = opts.ceiling();
    let counters = Arc::new(Counters::new());
    counters
        .bitrate
        .store(ceiling.bitrate as u64, Ordering::Relaxed);
    let _ = session.control_tx.send(Control::Quality {
        bitrate: ceiling.bitrate,
        fps: ceiling.fps,
        max_size: ceiling.max_size,
    });
    if !opts.no_window {
        println!("Alt+M menu, Alt+P pause, Alt+D disconnect");
    }
    let stats_stop = Arc::new(AtomicBool::new(false));

    let stop = session.stop_flag();
    let control_rx = session.take_control_rx().context("control rx")?;
    let epoch = Arc::new(Instant::now());
    let phone_stats = session.phone_stats_cell();
    let clock = session.clock_cell();
    let audio_on = Arc::new(AtomicBool::new(false));
    let audio_input_rate = Arc::new(AtomicU32::new(48_000));
    let net = {
        let stats_win = Arc::clone(&session.stats_window);
        let control_tx = session.control_tx.clone();
        let source = source.clone();
        let stop = Arc::clone(&stop);
        let counters = Arc::clone(&counters);
        let epoch = Arc::clone(&epoch);
        let audio_on = Arc::clone(&audio_on);
        let audio_input_rate = Arc::clone(&audio_input_rate);
        let phone_stats = Arc::clone(&phone_stats);
        let no_audio = opts.no_audio;
        thread::Builder::new()
            .stack_size(SMALL_STACK)
            .spawn(move || {
                network_loop(
                    stats_win,
                    control_tx,
                    control_rx,
                    Adaptive::new(ceiling),
                    source,
                    stop,
                    counters,
                    epoch,
                    audio_on,
                    audio_input_rate,
                    phone_stats,
                    no_audio,
                );
            })
            .expect("net thread")
    };

    let audio = if opts.no_audio {
        None
    } else {
        match AudioOutput::from_queue(
            Arc::clone(&session.audio),
            Arc::clone(&audio_on),
            Arc::clone(&stop),
            Arc::clone(&audio_input_rate),
        ) {
            Some(a) => Some(a),
            None => {
                tracing::warn!("no audio output device");
                None
            }
        }
    };

    let dec = {
        let frames = Arc::clone(&session.frames);
        let control_tx = session.control_tx.clone();
        let source = source.clone();
        let decoder = opts.decoder.clone();
        let stop = Arc::clone(&stop);
        let counters = Arc::clone(&counters);
        let phone_stats = Arc::clone(&phone_stats);
        let clock = Arc::clone(&clock);
        let epoch = Arc::clone(&epoch);
        thread::spawn(move || {
            decode_loop(
                frames,
                control_tx,
                source,
                decoder,
                stop,
                counters,
                phone_stats,
                clock,
                epoch,
            );
        })
    };

    let stats = (opts.stats || opts.verbose).then(|| {
        let counters = Arc::clone(&counters);
        let stats_win = Arc::clone(&session.stats_window);
        let frames = Arc::clone(&session.frames);
        let stop = Arc::clone(&stats_stop);
        let phone_stats = Arc::clone(&phone_stats);
        let clock = Arc::clone(&clock);
        let verbose = opts.verbose;
        thread::Builder::new()
            .stack_size(SMALL_STACK)
            .spawn(move || {
                stats_loop(
                    counters,
                    stats_win,
                    frames,
                    phone_stats,
                    clock,
                    stop,
                    verbose,
                )
            })
            .expect("stats thread")
    });

    while !stop.load(Ordering::Relaxed) {
        if session.take_rotate_request() {
            source.request_rotate();
        }
        thread::sleep(Duration::from_millis(50));
    }

    if let Some(a) = audio {
        a.stop();
    }
    stats_stop.store(true, Ordering::Relaxed);
    let _ = net.join();
    let _ = dec.join();
    if let Some(h) = stats {
        let _ = h.join();
    }
    let why = session.end_reason();
    session.join_transport();
    Ok(why)
}

#[allow(clippy::too_many_arguments)]
fn network_loop(
    stats_win: Arc<Mutex<krapy_transport::NetWindow>>,
    control_tx: Sender<Control>,
    control_rx: Receiver<Control>,
    mut adaptive: Adaptive,
    source: FrameSource,
    stop: Arc<AtomicBool>,
    counters: Arc<Counters>,
    epoch: Arc<Instant>,
    audio_on: Arc<AtomicBool>,
    audio_input_rate: Arc<AtomicU32>,
    phone_stats: Arc<Mutex<krapy_transport::PhoneStatsSnap>>,
    no_audio: bool,
) {
    static BAD_AUDIO_FMT: Once = Once::new();
    let mut last_stats = Instant::now();
    let mut prev_out = 0u64;
    let mut last_pause: Option<bool> = None;
    let mut menu_bitrate = counters.bitrate.load(Ordering::Relaxed) as u32;
    while !stop.load(Ordering::Relaxed) {
        while let Ok(msg) = control_rx.try_recv() {
            match msg {
                Control::Clipboard { text } => source.phone_clipboard(text),
                Control::PresetRequest { preset } => {
                    let p = preset_from_id(preset);
                    let q = p.quality();
                    adaptive.set_ceiling(q);
                    counters.bitrate.store(q.bitrate as u64, Ordering::Relaxed);
                    let _ = control_tx.send(Control::Quality {
                        bitrate: q.bitrate,
                        fps: q.fps,
                        max_size: q.max_size,
                    });
                }
                Control::Paused { on } => {
                    let paused = on != 0;
                    match last_pause {
                        Some(was) if was == paused => {}
                        _ => {
                            if paused {
                                println!("paused");
                            } else {
                                println!("resumed");
                            }
                        }
                    }
                    source.set_paused(paused);
                    last_pause = Some(paused);
                }
                Control::Settings { .. } => {
                    if let Some(s) = PhoneSettings::from_control(&msg) {
                        source.set_phone_settings(s);
                        if !no_audio {
                            audio_on.store(s.sound_pc, Ordering::Relaxed);
                        }
                        last_pause = Some(s.paused);
                    }
                }
                Control::AudioState {
                    on,
                    rate,
                    channels,
                    phone_muted,
                } if !no_audio => {
                    let want = on != 0;
                    if want {
                        let ok = matches!(rate, 48_000 | 44_100) && channels == 2;
                        if !ok {
                            BAD_AUDIO_FMT.call_once(|| {
                                tracing::warn!(
                                    "unsupported audio format {rate} Hz {channels} ch; audio off"
                                );
                            });
                            audio_on.store(false, Ordering::Relaxed);
                        } else {
                            audio_input_rate.store(rate, Ordering::Relaxed);
                            audio_on.store(true, Ordering::Relaxed);
                        }
                    } else {
                        audio_on.store(false, Ordering::Relaxed);
                    }
                    if want && phone_muted == 0 {
                        println!("phone could not mute its speaker; it may keep playing sound");
                    }
                }
                Control::PhoneStats { bitrate, .. } => {
                    menu_bitrate = bitrate;
                }
                _ => {}
            }
        }
        let now = Instant::now();
        if last_stats.elapsed() >= Duration::from_secs(1) {
            last_stats = now;
            let out_now = counters.frames_out.load(Ordering::Relaxed);
            let fps = out_now.saturating_sub(prev_out).min(u64::from(u8::MAX)) as u8;
            prev_out = out_now;
            let ds = counters.decode_samples.swap(0, Ordering::Relaxed).max(1);
            let decode_ms =
                (counters.decode_us.swap(0, Ordering::Relaxed) as f64 / ds as f64 / 1000.0) as u16;
            let lat_samples = counters.lat_samples.swap(0, Ordering::Relaxed).max(1);
            let latency_ms = (counters.lat_us.swap(0, Ordering::Relaxed) as f64
                / lat_samples as f64
                / 1000.0) as u16;
            let loss_permille = stats_win
                .lock()
                .ok()
                .and_then(|w| w.last())
                .map(|n| (n.loss * 1000.0).round() as u16)
                .unwrap_or(0);
            let _ = control_tx.send(Control::DesktopStats {
                fps,
                latency_ms,
                decode_ms,
                loss_permille,
            });
            let bitrate_bps = phone_stats
                .lock()
                .ok()
                .map(|p| p.bitrate)
                .unwrap_or(menu_bitrate);
            source.set_menu_stats(MenuStats {
                fps,
                latency_ms,
                decode_ms,
                bitrate_bps,
            });
            let _ = epoch;
        }
        if let Ok(mut nw) = stats_win.lock()
            && let Some(net) = nw.tick(now)
            && let Some(q) = adaptive.on_window(net, now)
        {
            counters.bitrate.store(q.bitrate as u64, Ordering::Relaxed);
            let _ = control_tx.send(Control::Quality {
                bitrate: q.bitrate,
                fps: q.fps,
                max_size: q.max_size,
            });
        }
        thread::sleep(NET_TICK);
    }
}

#[allow(clippy::too_many_arguments)]
fn decode_loop(
    frames: Arc<FrameQueue>,
    control_tx: Sender<Control>,
    source: FrameSource,
    decoder: Option<String>,
    stop: Arc<AtomicBool>,
    counters: Arc<Counters>,
    phone_stats: Arc<Mutex<krapy_transport::PhoneStatsSnap>>,
    clock: Arc<Mutex<krapy_transport::ClockSync>>,
    epoch: Arc<Instant>,
) {
    let mut dec: Box<dyn Decoder> = match open(decoder.as_deref()) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("no video decoder: {e}");
            return;
        }
    };
    let mut last_kf = Instant::now() - KEY_RETRY;
    let mut ask_key = |frames: &FrameQueue| {
        frames.broken();
        if last_kf.elapsed() >= KEY_RETRY {
            let _ = control_tx.send(Control::Keyframe);
            last_kf = Instant::now();
        }
    };
    while !stop.load(Ordering::Relaxed) {
        let Some(cf) = frames.pop_wait(POLL) else {
            continue;
        };
        counters.frames_in.fetch_add(1, Ordering::Relaxed);
        let t0 = Instant::now();
        match dec.decode(&cf.data) {
            Ok(Some(frame)) => {
                let us = t0.elapsed().as_micros() as u64;
                counters.decode_us.fetch_add(us, Ordering::Relaxed);
                counters.decode_samples.fetch_add(1, Ordering::Relaxed);
                counters.frames_out.fetch_add(1, Ordering::Relaxed);
                source.publish(frame);
                let lat = cf.received.elapsed().as_micros() as u64;
                counters.lat_us.fetch_add(lat, Ordering::Relaxed);
                counters.lat_samples.fetch_add(1, Ordering::Relaxed);
                counters.lat_max_us.fetch_max(lat, Ordering::Relaxed);
                let decoded_us = epoch.elapsed().as_micros() as u64;
                let offset = clock
                    .lock()
                    .ok()
                    .and_then(|c| c.median_offset_us())
                    .unwrap_or(0);
                let phone_enc = phone_stats.lock().ok().map(|p| p.enc_avg_us).unwrap_or(0);
                let capture_desktop = cf.capture_us as i64 - offset;
                let total = decoded_us as i64 - capture_desktop;
                if total > 0 {
                    counters.total_us.fetch_add(total as u64, Ordering::Relaxed);
                    counters.total_samples.fetch_add(1, Ordering::Relaxed);
                    counters
                        .total_max_us
                        .fetch_max(total as u64, Ordering::Relaxed);
                }
                let net = cf.first_arrival_us as i64 - capture_desktop - phone_enc as i64;
                if net > 0 {
                    counters.net_us.fetch_add(net as u64, Ordering::Relaxed);
                }
                let pipe = decoded_us as i64 - cf.completed_us as i64;
                if pipe > 0 {
                    counters
                        .pipe_decode_us
                        .fetch_add(pipe as u64, Ordering::Relaxed);
                }
            }
            Ok(None) => {
                if dec.needs_keyframe() && !frames.waiting_for_key() {
                    ask_key(&frames);
                }
            }
            Err(_) => {
                dec.reset();
                ask_key(&frames);
            }
        }
    }
}

fn stats_loop(
    counters: Arc<Counters>,
    stats_win: Arc<Mutex<krapy_transport::NetWindow>>,
    frames: Arc<FrameQueue>,
    phone_stats: Arc<Mutex<krapy_transport::PhoneStatsSnap>>,
    clock: Arc<Mutex<krapy_transport::ClockSync>>,
    stop: Arc<AtomicBool>,
    verbose: bool,
) {
    let mut prev_out = 0u64;
    while !stop.load(Ordering::Relaxed) {
        thread::sleep(Duration::from_secs(1));
        let out_now = counters.frames_out.load(Ordering::Relaxed);
        let fps = out_now.saturating_sub(prev_out);
        prev_out = out_now;
        let loss = stats_win
            .lock()
            .ok()
            .and_then(|w| w.last())
            .map(|net| net.loss)
            .unwrap_or(0.0);
        let ps = phone_stats.lock().ok().map(|p| *p).unwrap_or_default();
        let phone_ms = ps.enc_avg_us as f64 / 1000.0;
        let tn = counters.total_samples.swap(0, Ordering::Relaxed);
        let total_ms = if tn > 0 {
            counters.total_us.swap(0, Ordering::Relaxed) as f64 / tn as f64 / 1000.0
        } else {
            0.0
        };
        let total_max_ms = counters.total_max_us.swap(0, Ordering::Relaxed) as f64 / 1000.0;
        let net_ms = if tn > 0 {
            counters.net_us.swap(0, Ordering::Relaxed) as f64 / tn as f64 / 1000.0
        } else {
            0.0
        };
        let dec_ms = if tn > 0 {
            counters.pipe_decode_us.swap(0, Ordering::Relaxed) as f64 / tn as f64 / 1000.0
        } else {
            0.0
        };
        let ds = counters.decode_samples.load(Ordering::Relaxed).max(1);
        let decode_cpu_ms = counters.decode_us.load(Ordering::Relaxed) as f64 / ds as f64 / 1000.0;
        let skips = frames.skipped();
        let bitrate_mbps = counters.bitrate.load(Ordering::Relaxed) as f64 / 1_000_000.0;
        let has_clock = clock
            .lock()
            .ok()
            .and_then(|c| c.median_offset_us())
            .is_some();
        if has_clock && tn > 0 {
            println!(
                "latency  phone {phone_ms:.1} ms  network {net_ms:.1} ms  decode {dec_ms:.1} ms  dec_cpu {decode_cpu_ms:.2} ms  total {total_ms:.1} ms (capture to decoded; max {total_max_ms:.1} ms)  fps {fps}  {bitrate_mbps:.1} Mbps  loss {loss:.1}%{}",
                if verbose {
                    format!("  skipped {skips}")
                } else {
                    String::new()
                },
            );
        } else {
            println!(
                "latency  phone {phone_ms:.1} ms  (clock offset pending)  fps {fps}  {bitrate_mbps:.1} Mbps  loss {loss:.1}%{}",
                if verbose {
                    format!("  dec_cpu {decode_cpu_ms:.2} ms  skipped {skips}")
                } else {
                    String::new()
                },
            );
        }
    }
}

pub fn spawn_control_bridge() -> (Sender<Control>, ControlSlot) {
    let (ui_tx, ui_rx) = mpsc::channel::<Control>();
    let slot: ControlSlot = Arc::new(Mutex::new(None));
    let slot_c = Arc::clone(&slot);
    thread::spawn(move || {
        while let Ok(c) = ui_rx.recv() {
            if let Ok(g) = slot_c.lock()
                && let Some(tx) = g.as_ref()
            {
                let _ = tx.send(c);
            }
        }
    });
    (ui_tx, slot)
}

pub fn headless_connect(listener: Listener, store: Store, opts: MirrorOpts) -> Result<()> {
    let source = FrameSource::new();
    let slot: ControlSlot = Arc::new(Mutex::new(None));
    install_ctrlc();
    run_connect_loop(listener, store, source, slot, opts)
}

pub fn window_connect(
    listener: Listener,
    store: Store,
    title: &str,
    opts: MirrorOpts,
) -> Result<()> {
    let source = FrameSource::new();
    let (ui_tx, slot) = spawn_control_bridge();
    let source_c = source.clone();
    install_ctrlc();
    thread::spawn(move || {
        let _ = run_connect_loop(listener, store, source_c, slot, opts);
    });
    krapy_render::run(source, title, ui_tx)
}

pub fn headless_pair(
    listener: Listener,
    store: Store,
    k: [u8; 32],
    opts: MirrorOpts,
    phone_connected: Arc<AtomicBool>,
) -> Result<()> {
    let source = FrameSource::new();
    let slot: ControlSlot = Arc::new(Mutex::new(None));
    install_ctrlc();
    run_pair_then_connect(
        listener,
        store,
        source,
        slot,
        k,
        opts,
        Some(phone_connected),
    )
}

pub fn window_pair(
    listener: Listener,
    store: Store,
    k: [u8; 32],
    title: &str,
    pair_uri: &str,
    opts: MirrorOpts,
    phone_connected: Arc<AtomicBool>,
) -> Result<()> {
    let source = FrameSource::new();
    if let Ok(frame) = krapy_render::pair_qr_frame(pair_uri) {
        source.publish(frame);
    }
    let (ui_tx, slot) = spawn_control_bridge();
    let source_c = source.clone();
    let connected = Arc::clone(&phone_connected);
    install_ctrlc();
    thread::spawn(move || {
        let _ = run_pair_then_connect(listener, store, source_c, slot, k, opts, Some(connected));
    });
    krapy_render::run(source, title, ui_tx)
}

fn install_ctrlc() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let _ = ctrlc::set_handler(|| std::process::exit(0));
    });
}

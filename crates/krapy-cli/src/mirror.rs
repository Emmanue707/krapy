use std::sync::Once;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use krapy_core::adaptive::Adaptive;
use krapy_core::device::{PairedDevice, Store};
use krapy_core::preset::{Preset, Quality};
use krapy_decode::{Decoder, open};
use krapy_render::FrameSource;
use krapy_transport::{AcceptMode, Control, FrameQueue, Listener, Session, SessionError};

const PAIR_WINDOW: Duration = Duration::from_secs(300);
const POLL: Duration = Duration::from_millis(1);
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
        }
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
    let stats_stop = Arc::new(AtomicBool::new(false));

    let stop = session.stop_flag();
    let control_rx = session.take_control_rx().context("control rx")?;
    let net = {
        let stats_win = Arc::clone(&session.stats_window);
        let control_tx = session.control_tx.clone();
        let source = source.clone();
        let stop = Arc::clone(&stop);
        let counters = Arc::clone(&counters);
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
                );
            })
            .expect("net thread")
    };

    let dec = {
        let frames = Arc::clone(&session.frames);
        let control_tx = session.control_tx.clone();
        let source = source.clone();
        let decoder = opts.decoder.clone();
        let stop = Arc::clone(&stop);
        let counters = Arc::clone(&counters);
        thread::spawn(move || {
            decode_loop(frames, control_tx, source, decoder, stop, counters);
        })
    };

    let stats = (opts.stats || opts.verbose).then(|| {
        let counters = Arc::clone(&counters);
        let stats_win = Arc::clone(&session.stats_window);
        let frames = Arc::clone(&session.frames);
        let stop = Arc::clone(&stats_stop);
        thread::Builder::new()
            .stack_size(SMALL_STACK)
            .spawn(move || stats_loop(counters, stats_win, frames, stop))
            .expect("stats thread")
    });

    while !stop.load(Ordering::Relaxed) {
        thread::sleep(Duration::from_millis(50));
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
) {
    while !stop.load(Ordering::Relaxed) {
        while let Ok(msg) = control_rx.try_recv() {
            if let Control::Clipboard { text } = msg {
                source.phone_clipboard(text);
            }
        }
        let now = Instant::now();
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

fn decode_loop(
    frames: Arc<FrameQueue>,
    control_tx: Sender<Control>,
    source: FrameSource,
    decoder: Option<String>,
    stop: Arc<AtomicBool>,
    counters: Arc<Counters>,
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
        let Some(cf) = frames.pop() else {
            thread::sleep(POLL);
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
    stop: Arc<AtomicBool>,
) {
    let mut prev_in = 0u64;
    let mut prev_out = 0u64;
    while !stop.load(Ordering::Relaxed) {
        thread::sleep(Duration::from_secs(1));
        let in_now = counters.frames_in.load(Ordering::Relaxed);
        let out_now = counters.frames_out.load(Ordering::Relaxed);
        let fps_in = in_now.saturating_sub(prev_in);
        let fps_out = out_now.saturating_sub(prev_out);
        prev_in = in_now;
        prev_out = out_now;
        let (rtt, jitter, loss) = stats_win
            .lock()
            .ok()
            .and_then(|w| w.last())
            .map(|net| (net.rtt_ms, net.jitter_ms, net.loss))
            .unwrap_or((0.0, 0.0, 0.0));
        let ds = counters.decode_samples.load(Ordering::Relaxed).max(1);
        let decode_ms = counters.decode_us.load(Ordering::Relaxed) as f64 / ds as f64 / 1000.0;
        let bitrate = counters.bitrate.load(Ordering::Relaxed);
        let dropped = stats_win
            .lock()
            .ok()
            .map(|w| w.total_dropped())
            .unwrap_or(0);
        let n = counters.lat_samples.swap(0, Ordering::Relaxed);
        let sum = counters.lat_us.swap(0, Ordering::Relaxed);
        let max = counters.lat_max_us.swap(0, Ordering::Relaxed);
        let desk_ms = if n > 0 {
            sum as f64 / n as f64 / 1000.0
        } else {
            0.0
        };
        let skipped = frames.skipped();
        println!(
            "stats fps_in={fps_in} fps_out={fps_out} rtt={rtt:.1}ms jitter={jitter:.1}ms loss={loss:.2} bitrate={bitrate} dropped={dropped} skipped={skipped} decode={decode_ms:.1}ms",
        );
        println!(
            "latency ~{:.1}ms (network ~{:.1}ms + receive-to-screen {desk_ms:.1}ms avg, {:.1}ms max)",
            rtt as f64 / 2.0 + desk_ms,
            rtt as f64 / 2.0,
            max as f64 / 1000.0,
        );
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

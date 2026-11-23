use crate::crypto::{CryptoError, Opener, Sealer, SessionKeys, pair_secret};
use crate::queue::{AudioQueue, FrameQueue};
use crate::reassembly::Reassembler;
use crate::stats::{ClockSync, NetWindow, PhoneStatsSnap};
use crate::wire::{
    self, Control, Datagram, Hello, WireError, decode_control, encode_control,
    encode_datagram_body, encode_tcp_frame, encode_welcome,
};
use socket2::{Domain, SockAddr, Socket, Type};
use std::borrow::Cow;
use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream, UdpSocket};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);
const PING_INTERVAL: Duration = Duration::from_millis(250);
const UDP_SILENCE: Duration = Duration::from_secs(3);
const KEYFRAME_MIN: Duration = Duration::from_millis(300);

const END_NONE: u8 = 0;
const END_PHONE_CLOSED: u8 = 1;
const END_TCP_ERROR: u8 = 2;
const END_SILENT: u8 = 3;
const END_BYE: u8 = 4;

fn set_end(end: &AtomicU8, why: u8) {
    let _ = end.compare_exchange(END_NONE, why, Ordering::Relaxed, Ordering::Relaxed);
}

#[derive(Debug)]
pub enum SessionError {
    Io(io::Error),
    Wire(WireError),
    Crypto(CryptoError),
    HandshakeTimeout,
    UnknownPhone,
    BadFirstMessage,
}

impl From<io::Error> for SessionError {
    fn from(e: io::Error) -> Self {
        SessionError::Io(e)
    }
}

pub struct Session {
    pub control_tx: Sender<Control>,
    pub control_rx: Option<Receiver<Control>>,
    pub frames: Arc<FrameQueue>,
    pub audio: Arc<AudioQueue>,
    end: Arc<AtomicU8>,
    pub stats_window: Arc<Mutex<NetWindow>>,
    phone_stats: Arc<Mutex<PhoneStatsSnap>>,
    clock: Arc<Mutex<ClockSync>>,
    rotate: Arc<AtomicBool>,
    paused: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    handles: Mutex<Option<Vec<JoinHandle<()>>>>,
    phone_id: [u8; 8],
    phone_name: String,
}

impl Session {
    pub fn phone_id(&self) -> [u8; 8] {
        self.phone_id
    }

    pub fn phone_name(&self) -> &str {
        &self.phone_name
    }

    pub fn stopped(&self) -> bool {
        self.stop.load(Ordering::Relaxed)
    }

    pub fn end_reason(&self) -> &'static str {
        match self.end.load(Ordering::Relaxed) {
            END_PHONE_CLOSED => "the phone closed the connection",
            END_TCP_ERROR => "connection error",
            END_SILENT => "nothing from the phone for 3s",
            END_BYE => "the phone stopped mirroring",
            _ => "stopped",
        }
    }

    pub fn stop_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.stop)
    }

    pub fn stop(self) {
        self.join_transport();
    }

    pub fn take_control_rx(&mut self) -> Option<Receiver<Control>> {
        self.control_rx.take()
    }

    pub fn take_rotate_request(&self) -> bool {
        self.rotate.swap(false, Ordering::Relaxed)
    }

    pub fn phone_stats(&self) -> PhoneStatsSnap {
        self.phone_stats.lock().map(|g| *g).unwrap_or_default()
    }

    pub fn clock_offset_us(&self) -> Option<i64> {
        self.clock.lock().ok().and_then(|c| c.median_offset_us())
    }

    pub fn phone_stats_cell(&self) -> Arc<Mutex<PhoneStatsSnap>> {
        Arc::clone(&self.phone_stats)
    }

    pub fn clock_cell(&self) -> Arc<Mutex<ClockSync>> {
        Arc::clone(&self.clock)
    }

    pub fn paused_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.paused)
    }

    pub fn is_paused(&self) -> bool {
        self.paused.load(Ordering::Relaxed)
    }

    pub fn join_transport(self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Ok(mut h) = self.handles.lock()
            && let Some(handles) = h.take()
        {
            for j in handles {
                let _ = j.join();
            }
        }
        let _ = self.control_rx;
    }
}

pub enum AcceptMode<'a> {
    Pair {
        k: [u8; 32],
        on_paired: &'a mut dyn FnMut([u8; 8], [u8; 32]),
    },
    Known {
        lookup: &'a dyn Fn([u8; 8]) -> Option<[u8; 32]>,
    },
}

pub struct Listener {
    tcp: TcpListener,
    udp: UdpSocket,
    desktop_id: [u8; 8],
    port: u16,
}

fn tune_tos(sock: &Socket) {
    #[cfg(unix)]
    {
        let _ = sock.set_tos(0xB8);
    }
}

impl Listener {
    pub fn bind(port: u16, desktop_id: [u8; 8]) -> io::Result<Self> {
        let addr: SocketAddr = format!("0.0.0.0:{port}").parse().expect("addr");
        let sock_addr = SockAddr::from(addr);
        let tcp_sock = Socket::new(Domain::IPV4, Type::STREAM, None)?;
        tcp_sock.set_reuse_address(true)?;
        tcp_sock.bind(&sock_addr)?;
        tcp_sock.listen(128)?;
        tune_tos(&tcp_sock);
        let tcp: TcpListener = tcp_sock.into();

        let udp_sock = Socket::new(Domain::IPV4, Type::DGRAM, None)?;
        udp_sock.set_reuse_address(true)?;
        udp_sock.bind(&sock_addr)?;
        let _ = udp_sock.set_recv_buffer_size(4 * 1024 * 1024);
        tune_tos(&udp_sock);
        let udp: UdpSocket = udp_sock.into();
        udp.set_read_timeout(Some(Duration::from_millis(50)))?;
        Ok(Self {
            tcp,
            udp,
            desktop_id,
            port,
        })
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn local_udp(&self) -> &UdpSocket {
        &self.udp
    }

    pub fn accept(&self, mode: AcceptMode<'_>) -> Result<Session, SessionError> {
        self.tcp.set_nonblocking(true)?;
        let deadline = Instant::now() + HANDSHAKE_TIMEOUT;
        let mut stream = loop {
            match self.tcp.accept() {
                Ok((s, _)) => break s,
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                    if Instant::now() >= deadline {
                        return Err(SessionError::HandshakeTimeout);
                    }
                    thread::sleep(Duration::from_millis(10));
                }
                Err(e) => return Err(SessionError::Io(e)),
            }
        };
        self.tcp.set_nonblocking(false)?;
        if let Ok(clone) = stream.try_clone() {
            tune_tos(&Socket::from(clone));
        }
        stream.set_read_timeout(Some(HANDSHAKE_TIMEOUT))?;
        stream.set_write_timeout(Some(HANDSHAKE_TIMEOUT))?;

        let hello = read_plain_hello(&mut stream)?;
        let s = resolve_secret(&hello.phone_id, mode)?;
        let mut nonce_d = [0u8; 16];
        rand::Rng::fill(&mut rand::rng(), &mut nonce_d);
        let welcome = wire::Welcome {
            desktop_id: self.desktop_id,
            nonce_d,
        };
        write_tcp_plain(&mut stream, &encode_welcome(&welcome))?;

        let keys = SessionKeys::derive(&s, &hello.nonce_p, &nonce_d);
        let start_pt = read_sealed_start(&mut stream, &keys)?;
        if !matches!(start_pt, Control::Start { .. }) {
            return Err(SessionError::BadFirstMessage);
        }

        let udp = self.udp.try_clone()?;
        let (control_tx, control_in) = mpsc::channel();
        let (control_out, control_rx) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let (auto_tx, auto_rx) = mpsc::channel();
        let frames = Arc::new(FrameQueue::new());
        let audio = Arc::new(AudioQueue::new());
        let end = Arc::new(AtomicU8::new(END_NONE));
        let stats_window = Arc::new(Mutex::new(NetWindow::new()));
        let phone_stats = Arc::new(Mutex::new(PhoneStatsSnap::default()));
        let clock = Arc::new(Mutex::new(ClockSync::default()));
        let rotate = Arc::new(AtomicBool::new(false));
        let paused = Arc::new(AtomicBool::new(false));

        let _ = stream.set_nodelay(true);
        let phone_addr = Arc::new(Mutex::new(None::<SocketAddr>));
        let last_udp = Arc::new(Mutex::new(Instant::now()));

        let tcp_h = {
            let stop = Arc::clone(&stop);
            let tcp_video = TcpVideo {
                frames: Arc::clone(&frames),
                audio: Arc::clone(&audio),
                last_in: Arc::clone(&last_udp),
                auto_tx: auto_tx.clone(),
                reasm: Reassembler::new(),
                last_key_req: Instant::now() - KEYFRAME_MIN,
                epoch: Instant::now(),
            };
            let end = Arc::clone(&end);
            let phone_stats = Arc::clone(&phone_stats);
            let rotate = Arc::clone(&rotate);
            let paused = Arc::clone(&paused);
            thread::spawn(move || {
                tcp_loop(
                    stream,
                    keys.tcp_p2d,
                    keys.tcp_d2p,
                    control_in,
                    auto_rx,
                    control_out,
                    tcp_video,
                    phone_stats,
                    rotate,
                    paused,
                    stop,
                    end,
                )
            })
        };

        let udp_h = {
            let udp = udp;
            let phone_addr = Arc::clone(&phone_addr);
            let frames = Arc::clone(&frames);
            let audio = Arc::clone(&audio);
            let stats = Arc::clone(&stats_window);
            let clock = Arc::clone(&clock);
            let last_udp = Arc::clone(&last_udp);
            let stop = Arc::clone(&stop);
            let auto_tx = auto_tx;
            thread::spawn(move || {
                udp_loop(
                    udp,
                    keys.udp_p2d,
                    keys.udp_d2p,
                    phone_addr,
                    last_udp,
                    frames,
                    audio,
                    stats,
                    clock,
                    auto_tx,
                    stop,
                )
            })
        };

        let watch_h = {
            let last_udp = Arc::clone(&last_udp);
            let paused = Arc::clone(&paused);
            let stop = Arc::clone(&stop);
            let end = Arc::clone(&end);
            thread::spawn(move || watch_loop(last_udp, paused, stop, end))
        };

        let stop_tx = stop;
        Ok(Session {
            control_tx,
            control_rx: Some(control_rx),
            frames,
            audio,
            end,
            stats_window,
            phone_stats,
            clock,
            rotate,
            paused,
            stop: stop_tx,
            handles: Mutex::new(Some(vec![tcp_h, udp_h, watch_h])),
            phone_id: hello.phone_id,
            phone_name: hello.name,
        })
    }
}

fn resolve_secret(phone_id: &[u8; 8], mode: AcceptMode<'_>) -> Result<[u8; 32], SessionError> {
    match mode {
        AcceptMode::Pair { k, on_paired } => {
            let s = pair_secret(&k);
            on_paired(*phone_id, s);
            Ok(s)
        }
        AcceptMode::Known { lookup } => lookup(*phone_id).ok_or(SessionError::UnknownPhone),
    }
}

fn read_plain_hello(stream: &mut TcpStream) -> Result<Hello, SessionError> {
    let mut len_buf = [0u8; 2];
    stream.read_exact(&mut len_buf)?;
    let len = u16::from_be_bytes(len_buf) as usize;
    let mut body = vec![0u8; len];
    stream.read_exact(&mut body)?;
    wire::decode_hello(&body).map_err(SessionError::Wire)
}

fn write_tcp_plain(stream: &mut TcpStream, body: &[u8]) -> Result<(), SessionError> {
    let frame = encode_tcp_frame(body);
    stream.write_all(&frame)?;
    stream.flush()?;
    Ok(())
}

fn read_sealed_start(stream: &mut TcpStream, keys: &SessionKeys) -> Result<Control, SessionError> {
    let mut len_buf = [0u8; 2];
    stream.read_exact(&mut len_buf)?;
    let len = u16::from_be_bytes(len_buf) as usize;
    let mut wire = vec![0u8; len];
    stream.read_exact(&mut wire)?;
    let mut opener = Opener::new(&keys.tcp_p2d);
    let pt = opener.open_tcp(&wire).map_err(SessionError::Crypto)?;
    decode_control(&pt).map_err(SessionError::Wire)
}

struct TcpVideo {
    frames: Arc<FrameQueue>,
    audio: Arc<AudioQueue>,
    last_in: Arc<Mutex<LastUdp>>,
    auto_tx: Sender<Control>,
    reasm: Reassembler,
    last_key_req: Instant,
    epoch: Instant,
}

impl TcpVideo {
    fn touch(&self) {
        if let Ok(mut t) = self.last_in.lock() {
            *t = Instant::now();
        }
    }

    fn ingest(&mut self, c: &Control) {
        let Control::VideoChunk {
            frame_id,
            idx,
            count,
            flags,
            capture_us,
            data,
        } = c
        else {
            return;
        };
        let dg = Datagram::Video {
            frame_id: *frame_id,
            idx: *idx,
            count: *count,
            flags: *flags,
            capture_us: *capture_us,
            data: Cow::Borrowed(data),
        };
        let arrival_us = self.epoch.elapsed().as_micros() as u64;
        let mut want_key = false;
        if let Some(cf) = self.reasm.ingest(&dg, arrival_us) {
            want_key = self.frames.push(cf);
        }
        if (want_key || self.reasm.keyframe_wanted()) && self.last_key_req.elapsed() >= KEYFRAME_MIN
        {
            let _ = self.auto_tx.send(Control::Keyframe);
            self.last_key_req = Instant::now();
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn tcp_loop(
    mut stream: TcpStream,
    read_key: [u8; 32],
    write_key: [u8; 32],
    user_in: Receiver<Control>,
    auto_in: Receiver<Control>,
    out: Sender<Control>,
    mut video: TcpVideo,
    phone_stats: Arc<Mutex<PhoneStatsSnap>>,
    rotate: Arc<AtomicBool>,
    paused: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    end: Arc<AtomicU8>,
) {
    let writer = match stream.try_clone() {
        Ok(w) => Arc::new(Mutex::new((w, Sealer::new(&write_key)))),
        Err(_) => {
            stop.store(true, Ordering::Relaxed);
            return;
        }
    };
    for (rx, user) in [(user_in, true), (auto_in, false)] {
        let writer = Arc::clone(&writer);
        let stop = Arc::clone(&stop);
        thread::spawn(move || {
            for c in rx {
                if stop.load(Ordering::Relaxed) || send_control(&writer, &c).is_err() {
                    stop.store(true, Ordering::Relaxed);
                    return;
                }
            }
            if user {
                stop.store(true, Ordering::Relaxed);
            }
        });
    }

    let mut opener = Opener::new(&read_key);
    let _ = stream.set_read_timeout(Some(Duration::from_millis(100)));
    let mut pending = Vec::with_capacity(64 * 1024);
    let mut buf = vec![0u8; 64 * 1024];
    while !stop.load(Ordering::Relaxed) {
        match stream.read(&mut buf) {
            Ok(0) => {
                set_end(&end, END_PHONE_CLOSED);
                break;
            }
            Ok(n) => pending.extend_from_slice(&buf[..n]),
            Err(e)
                if e.kind() == io::ErrorKind::WouldBlock || e.kind() == io::ErrorKind::TimedOut =>
            {
                continue;
            }
            Err(_) => {
                set_end(&end, END_TCP_ERROR);
                break;
            }
        }
        while pending.len() >= 2 {
            let len = u16::from_be_bytes([pending[0], pending[1]]) as usize;
            if pending.len() < 2 + len {
                break;
            }
            if let Ok(pt) = opener.open_tcp(&pending[2..2 + len])
                && let Ok(msg) = decode_control(&pt)
            {
                video.touch();
                match &msg {
                    Control::VideoChunk { .. } => video.ingest(&msg),
                    Control::AudioChunk { seq, pts_us, data } => {
                        video.audio.push(*seq, *pts_us, data);
                    }
                    Control::PhoneStats {
                        enc_avg_us,
                        enc_max_us,
                        fps,
                        bitrate,
                    } => {
                        if let Ok(mut g) = phone_stats.lock() {
                            *g = PhoneStatsSnap {
                                enc_avg_us: *enc_avg_us,
                                enc_max_us: *enc_max_us,
                                fps: *fps,
                                bitrate: *bitrate,
                            };
                        }
                        video.touch();
                    }
                    Control::Paused { on } => {
                        paused.store(*on != 0, Ordering::Relaxed);
                        let _ = out.send(msg);
                    }
                    Control::RotateView => {
                        rotate.store(true, Ordering::Relaxed);
                    }
                    Control::PresetRequest { .. } | Control::AudioState { .. } => {
                        let _ = out.send(msg);
                    }
                    Control::Bye => {
                        set_end(&end, END_BYE);
                        let _ = out.send(msg);
                    }
                    _ => {
                        let _ = out.send(msg);
                    }
                }
            }
            pending.drain(..2 + len);
        }
    }
    stop.store(true, Ordering::Relaxed);
    let _ = stream.shutdown(std::net::Shutdown::Both);
}

fn send_control(writer: &Mutex<(TcpStream, Sealer)>, ctrl: &Control) -> Result<(), ()> {
    let body = encode_control(ctrl).map_err(|_| ())?;
    let mut w = writer.lock().map_err(|_| ())?;
    let (stream, sealer) = &mut *w;
    let frame = encode_tcp_frame(&sealer.seal_tcp(&body));
    stream.write_all(&frame).map_err(|_| ())
}

#[allow(clippy::too_many_arguments)]
fn udp_loop(
    udp: UdpSocket,
    in_key: [u8; 32],
    out_key: [u8; 32],
    phone_addr: Arc<Mutex<Option<SocketAddr>>>,
    last_udp: Arc<Mutex<LastUdp>>,
    frames: Arc<FrameQueue>,
    audio: Arc<AudioQueue>,
    stats: Arc<Mutex<NetWindow>>,
    clock: Arc<Mutex<ClockSync>>,
    auto_tx: Sender<Control>,
    stop: Arc<AtomicBool>,
) {
    let mut opener = Opener::new(&in_key);
    let mut ping_sealer = Sealer::new(&out_key);
    let mut reasm = Reassembler::new();
    let mut last_keyframe = Instant::now() - KEYFRAME_MIN;
    let mut last_ping = Instant::now() - PING_INTERVAL;
    let mut ping_id: u32 = 0;
    let mut queue_wants_key = false;
    let epoch = Instant::now();
    let mut buf = [0u8; 2048];
    loop {
        if stop.load(Ordering::Relaxed) {
            break;
        }
        let now = Instant::now();
        if last_ping.elapsed() >= PING_INTERVAL {
            if let Some(addr) = phone_addr.lock().ok().and_then(|g| *g) {
                ping_id = ping_id.wrapping_add(1);
                let t_us = now.duration_since(epoch).as_micros() as u64;
                let ping = Datagram::Ping { id: ping_id, t_us };
                let _ = send_udp(&udp, &mut ping_sealer, addr, &ping);
            }
            last_ping = now;
        }
        match udp.recv_from(&mut buf) {
            Ok((n, from)) => {
                if n < 9 + 16 {
                    continue;
                }
                let wire = &buf[..n];
                let ty = wire[0];
                let pt = match try_open_udp(&mut opener, wire, &phone_addr, from, in_key) {
                    Some(p) => p,
                    None => continue,
                };
                if let Ok(mut g) = phone_addr.lock() {
                    *g = Some(from);
                }
                if let Ok(mut t) = last_udp.lock() {
                    *t = Instant::now();
                }
                let mut full = Vec::with_capacity(1 + pt.len());
                full.push(ty);
                full.extend_from_slice(&pt);
                let dg = match wire::decode_datagram_body(&full) {
                    Ok(d) => d,
                    Err(_) => continue,
                };
                match dg {
                    Datagram::Video {
                        capture_us,
                        ref data,
                        ..
                    } => {
                        let arrival_us = epoch.elapsed().as_micros() as u64;
                        if let Ok(mut nw) = stats.lock() {
                            nw.note_video_arrival(capture_us, arrival_us);
                        }
                        let prev_drops = reasm.stats().frames_dropped;
                        let prev_ok = reasm.stats().frames_ok;
                        if let Some(cf) = reasm.ingest(&dg, arrival_us) {
                            if reasm.stats().frames_ok > prev_ok
                                && let Ok(mut nw) = stats.lock()
                            {
                                nw.note_frame_completed();
                            }
                            if frames.push(cf) {
                                queue_wants_key = true;
                            }
                        }
                        let new_drops = reasm.stats().frames_dropped;
                        if new_drops > prev_drops {
                            let delta = new_drops - prev_drops;
                            if let Ok(mut nw) = stats.lock() {
                                for _ in 0..delta {
                                    nw.note_frame_dropped();
                                }
                            }
                        }
                        let _ = data;
                    }
                    Datagram::Pong { id, t_us, phone_us } => {
                        let now_us = epoch.elapsed().as_micros() as u64;
                        if now_us >= t_us {
                            let rtt = Duration::from_micros(now_us - t_us);
                            if let Ok(mut nw) = stats.lock() {
                                nw.note_ping_rtt(rtt);
                            }
                        }
                        if let (Some(pu), Ok(mut cl)) = (phone_us, clock.lock()) {
                            cl.note_pong(t_us, pu, now_us);
                        }
                        let _ = id;
                    }
                    Datagram::Audio {
                        seq,
                        pts_us,
                        ref data,
                        ..
                    } => {
                        audio.push(seq, pts_us, data);
                    }
                    Datagram::Keepalive | Datagram::Ping { .. } => {}
                }
                if (reasm.keyframe_wanted() || queue_wants_key)
                    && last_keyframe.elapsed() >= KEYFRAME_MIN
                {
                    queue_wants_key = false;
                    let _ = auto_tx.send(Control::Keyframe);
                    last_keyframe = Instant::now();
                }
            }
            Err(e)
                if e.kind() == io::ErrorKind::WouldBlock || e.kind() == io::ErrorKind::TimedOut => {
            }
            Err(_) => break,
        }
    }
    stop.store(true, Ordering::Relaxed);
}

type LastUdp = Instant;

fn try_open_udp(
    opener: &mut Opener,
    wire: &[u8],
    phone_addr: &Arc<Mutex<Option<SocketAddr>>>,
    from: SocketAddr,
    key: [u8; 32],
) -> Option<Vec<u8>> {
    let known = phone_addr.lock().ok().and_then(|g| *g);
    if let Some(k) = known
        && from != k
    {
        let mut trial = Opener::new(&key);
        if trial.open_udp(wire).is_err() {
            return None;
        }
    }
    opener.open_udp(wire).ok()
}

fn send_udp(
    udp: &UdpSocket,
    sealer: &mut Sealer,
    addr: SocketAddr,
    d: &Datagram<'_>,
) -> Result<(), ()> {
    let body = encode_datagram_body(d).map_err(|_| ())?;
    let mut header = [0u8; 9];
    header[0] = body[0];
    header[1..9].copy_from_slice(&sealer.counter().to_be_bytes());
    let wire = sealer.seal_udp(&header, &body[1..]);
    udp.send_to(&wire, addr).map(|_| ()).map_err(|_| ())
}

fn watch_loop(
    last_udp: Arc<Mutex<LastUdp>>,
    paused: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    end: Arc<AtomicU8>,
) {
    loop {
        if stop.load(Ordering::Relaxed) {
            break;
        }
        if paused.load(Ordering::Relaxed) {
            thread::sleep(Duration::from_millis(100));
            continue;
        }
        let silent = last_udp
            .lock()
            .ok()
            .map(|t| t.elapsed() > UDP_SILENCE)
            .unwrap_or(false);
        if silent {
            set_end(&end, END_SILENT);
            stop.store(true, Ordering::Relaxed);
            break;
        }
        thread::sleep(Duration::from_millis(100));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reassembly::FLAG_KEYFRAME;
    use crate::wire::encode_hello;
    use std::borrow::Cow;

    fn pick_port() -> u16 {
        UdpSocket::bind("127.0.0.1:0")
            .expect("u")
            .local_addr()
            .expect("a")
            .port()
    }

    #[test]
    fn loopback_handshake_video_keyframe_rtt() {
        let port = pick_port();
        let desktop_id = [0xde, 0xad, 0xbe, 0xef, 0, 0, 0, 1];
        let listener = Listener::bind(port, desktop_id).expect("bind");
        let phone_id = [1u8; 8];
        let k = [9u8; 32];
        let paired = Arc::new(Mutex::new(None));
        let paired_c = Arc::clone(&paired);
        let accept_h = thread::spawn(move || {
            let mut cb = |pid: [u8; 8], s: [u8; 32]| {
                *paired_c.lock().expect("l") = Some((pid, s));
            };
            listener.accept(AcceptMode::Pair {
                k,
                on_paired: &mut cb,
            })
        });
        thread::sleep(Duration::from_millis(30));
        let dest = format!("127.0.0.1:{port}");
        let mut tcp = TcpStream::connect(&dest).expect("tcp");
        let udp = UdpSocket::bind("127.0.0.1:0").expect("udp");
        let mut nonce_p = [0u8; 16];
        rand::Rng::fill(&mut rand::rng(), &mut nonce_p);
        let hello = Hello {
            phone_id,
            nonce_p,
            name: "test".into(),
        };
        tcp.write_all(&encode_tcp_frame(&encode_hello(&hello).expect("h")))
            .expect("hello");
        let mut len = [0u8; 2];
        tcp.read_exact(&mut len).expect("rl");
        let l = u16::from_be_bytes(len) as usize;
        let mut body = vec![0u8; l];
        tcp.read_exact(&mut body).expect("rb");
        let welcome = wire::decode_welcome(&body).expect("w");
        let s = pair_secret(&k);
        let keys = SessionKeys::derive(&s, &nonce_p, &welcome.nonce_d);
        let start = encode_control(&Control::Start {
            width: 1280,
            height: 720,
            fps: 30,
            bitrate: 2_000_000,
            codec: 1,
        })
        .expect("st");
        let mut tseal = Sealer::new(&keys.tcp_p2d);
        tcp.write_all(&encode_tcp_frame(&tseal.seal_tcp(&start)))
            .expect("start");

        let session = accept_h.join().expect("j").expect("acc");
        assert_eq!(session.phone_id(), phone_id);
        assert!(paired.lock().expect("p").is_some());

        let mut useal = Sealer::new(&keys.udp_p2d);
        send_phone(&udp, &dest, &mut useal, &Datagram::Keepalive);

        thread::sleep(Duration::from_millis(80));

        send_phone(
            &udp,
            &dest,
            &mut useal,
            &Datagram::Video {
                frame_id: 1,
                idx: 0,
                count: 2,
                flags: FLAG_KEYFRAME,
                capture_us: 1_000_000,
                data: Cow::Borrowed(b"ab"),
            },
        );
        send_phone(
            &udp,
            &dest,
            &mut useal,
            &Datagram::Video {
                frame_id: 1,
                idx: 1,
                count: 2,
                flags: FLAG_KEYFRAME,
                capture_us: 1_000_033,
                data: Cow::Borrowed(b"cd"),
            },
        );

        let mut f1 = None;
        for _ in 0..50 {
            if let Some(f) = session.frames.pop() {
                f1 = Some(f);
                break;
            }
            thread::sleep(Duration::from_millis(20));
        }
        let f1 = f1.expect("expected assembled keyframe");
        assert_eq!(f1.data, b"abcd");

        send_phone(
            &udp,
            &dest,
            &mut useal,
            &Datagram::Video {
                frame_id: 2,
                idx: 0,
                count: 2,
                flags: 0,
                capture_us: 2_000_000,
                data: Cow::Borrowed(b"xx"),
            },
        );
        send_phone(
            &udp,
            &dest,
            &mut useal,
            &Datagram::Video {
                frame_id: 3,
                idx: 0,
                count: 1,
                flags: 0,
                capture_us: 3_000_000,
                data: Cow::Borrowed(b"yy"),
            },
        );

        let kf_h = {
            let mut tcp = tcp.try_clone().expect("clone");
            let rkey = keys.tcp_d2p;
            thread::spawn(move || {
                let mut opener = Opener::new(&rkey);
                for _ in 0..40 {
                    let mut len = [0u8; 2];
                    if tcp.read_exact(&mut len).is_err() {
                        break;
                    }
                    let l = u16::from_be_bytes(len) as usize;
                    let mut wire = vec![0u8; l];
                    if tcp.read_exact(&mut wire).is_err() {
                        break;
                    }
                    if let Ok(pt) = opener.open_tcp(&wire)
                        && matches!(decode_control(&pt), Ok(Control::Keyframe))
                    {
                        return true;
                    }
                    thread::sleep(Duration::from_millis(5));
                }
                false
            })
        };
        let mut popen = Opener::new(&keys.udp_d2p);
        udp.set_read_timeout(Some(Duration::from_secs(2)))
            .expect("timeout");
        let mut rbuf = [0u8; 2048];
        let (id, t_us) = loop {
            let n = udp.recv(&mut rbuf).expect("ping");
            let mut body = vec![rbuf[0]];
            body.extend(popen.open_udp(&rbuf[..n]).expect("open"));
            if let Ok(Datagram::Ping { id, t_us }) = wire::decode_datagram_body(&body) {
                break (id, t_us);
            }
        };
        thread::sleep(Duration::from_millis(5));
        send_phone(
            &udp,
            &dest,
            &mut useal,
            &Datagram::Pong {
                id,
                t_us,
                phone_us: None,
            },
        );
        thread::sleep(Duration::from_millis(300));
        assert!(session.stats_window.lock().expect("st").rtt_ms() >= 5.0);
        assert!(kf_h.join().expect("kf"));
        session.stop();
    }

    fn send_phone(udp: &UdpSocket, dest: &str, sealer: &mut Sealer, d: &Datagram<'_>) {
        let body = encode_datagram_body(d).expect("b");
        let mut header = [0u8; 9];
        header[0] = body[0];
        header[1..9].copy_from_slice(&sealer.counter().to_be_bytes());
        let wire = sealer.seal_udp(&header, &body[1..]);
        udp.send_to(&wire, dest).expect("s");
    }
}

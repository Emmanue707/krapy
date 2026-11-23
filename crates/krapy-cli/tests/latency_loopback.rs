// fake phone on loopback: real 720p h264 in, decoded frames out, time measured on one clock
use std::borrow::Cow;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpStream, UdpSocket};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use krapy_decode::open;
use krapy_transport::crypto::{Sealer, SessionKeys, pair_secret};
use krapy_transport::reassembly::FLAG_KEYFRAME;
use krapy_transport::wire::{
    Control, Datagram, Hello, encode_control, encode_datagram_body, encode_hello, encode_tcp_frame,
};
use krapy_transport::{AcceptMode, Listener, Session};

const CLIP: &[u8] = include_bytes!("data/lat720.h264");
const UDP_CHUNK: usize = 1100;
const TCP_CHUNK: usize = 32 * 1024;

fn access_units(data: &[u8]) -> Vec<(bool, Vec<u8>)> {
    let mut starts = Vec::new();
    let mut i = 0;
    while i + 4 < data.len() {
        if data[i..].starts_with(&[0, 0, 0, 1]) {
            starts.push((i, data[i + 4] & 0x1f));
            i += 4;
        } else if data[i..].starts_with(&[0, 0, 1]) {
            starts.push((i, data[i + 3] & 0x1f));
            i += 3;
        } else {
            i += 1;
        }
    }
    let mut aus = Vec::new();
    let mut cur = Vec::new();
    let mut key = false;
    let mut has_vcl = false;
    for (k, &(start, ty)) in starts.iter().enumerate() {
        let end = starts.get(k + 1).map(|s| s.0).unwrap_or(data.len());
        let nal = &data[start..end];
        let hdr = if nal.starts_with(&[0, 0, 0, 1]) { 4 } else { 3 };
        let vcl = ty == 1 || ty == 5;
        // first_mb_in_slice == 0 starts a new picture
        let first_slice = vcl && nal.get(hdr + 1).is_some_and(|b| b & 0x80 != 0);
        if (first_slice || (!vcl && has_vcl)) && has_vcl {
            aus.push((key, std::mem::take(&mut cur)));
            key = false;
            has_vcl = false;
        }
        cur.extend_from_slice(nal);
        key |= ty == 5;
        has_vcl |= vcl;
    }
    if has_vcl {
        aus.push((key, cur));
    }
    aus
}

struct Phone {
    tcp: TcpStream,
    udp: UdpSocket,
    dest: String,
    tseal: Sealer,
    useal: Sealer,
    session: Session,
}

fn connect() -> anyhow::Result<Phone> {
    let port = UdpSocket::bind("127.0.0.1:0")?.local_addr()?.port();
    let listener = Listener::bind(port, [0xde, 0xad, 0xbe, 0xef, 0, 0, 0, 2])?;
    let k = [0x33u8; 32];
    let accept_h = thread::spawn(move || {
        let mut cb = |_: [u8; 8], _: [u8; 32]| {};
        listener.accept(AcceptMode::Pair {
            k,
            on_paired: &mut cb,
        })
    });
    thread::sleep(Duration::from_millis(20));
    let dest = format!("127.0.0.1:{port}");
    let mut tcp = TcpStream::connect(&dest)?;
    tcp.set_nodelay(true)?;
    let mut nonce_p = [0u8; 16];
    rand::Rng::fill(&mut rand::rng(), &mut nonce_p);
    let hello = Hello {
        phone_id: [0x44; 8],
        nonce_p,
        name: "lat".into(),
    };
    tcp.write_all(&encode_tcp_frame(
        &encode_hello(&hello).map_err(|e| anyhow::anyhow!("{e:?}"))?,
    ))?;
    let mut len = [0u8; 2];
    tcp.read_exact(&mut len)?;
    let mut body = vec![0u8; u16::from_be_bytes(len) as usize];
    tcp.read_exact(&mut body)?;
    let welcome =
        krapy_transport::wire::decode_welcome(&body).map_err(|e| anyhow::anyhow!("{e:?}"))?;
    let keys = SessionKeys::derive(&pair_secret(&k), &nonce_p, &welcome.nonce_d);
    let start = encode_control(&Control::Start {
        width: 1280,
        height: 720,
        fps: 60,
        bitrate: 3_000_000,
        codec: 1,
    })
    .map_err(|e| anyhow::anyhow!("{e:?}"))?;
    let mut tseal = Sealer::new(&keys.tcp_p2d);
    tcp.write_all(&encode_tcp_frame(&tseal.seal_tcp(&start)))?;
    let session = accept_h
        .join()
        .map_err(|_| anyhow::anyhow!("accept thread"))?
        .map_err(|e| anyhow::anyhow!("{e:?}"))?;
    let udp = UdpSocket::bind("127.0.0.1:0")?;
    Ok(Phone {
        tcp,
        udp,
        dest,
        tseal,
        useal: Sealer::new(&keys.udp_p2d),
        session,
    })
}

impl Phone {
    fn send_udp(&mut self, id: u32, key: bool, au: &[u8]) {
        let count = au.len().div_ceil(UDP_CHUNK) as u16;
        for idx in 0..count {
            let s = idx as usize * UDP_CHUNK;
            let d = Datagram::Video {
                frame_id: id,
                idx,
                count,
                flags: if key { FLAG_KEYFRAME } else { 0 },
                capture_us: id as u64 * 16_667,
                data: Cow::Borrowed(&au[s..(s + UDP_CHUNK).min(au.len())]),
            };
            let body = encode_datagram_body(&d).expect("body");
            let mut header = [0u8; 9];
            header[0] = body[0];
            header[1..9].copy_from_slice(&self.useal.counter().to_be_bytes());
            let wire = self.useal.seal_udp(&header, &body[1..]);
            self.udp.send_to(&wire, &self.dest).expect("send");
        }
    }

    fn send_tcp(&mut self, id: u32, key: bool, au: &[u8]) {
        let count = au.len().div_ceil(TCP_CHUNK) as u16;
        for idx in 0..count {
            let s = idx as usize * TCP_CHUNK;
            let c = Control::VideoChunk {
                frame_id: id,
                idx,
                count,
                flags: if key { FLAG_KEYFRAME } else { 0 },
                capture_us: id as u64 * 16_667,
                data: au[s..(s + TCP_CHUNK).min(au.len())].to_vec(),
            };
            let pt = encode_control(&c).expect("chunk");
            let frame = encode_tcp_frame(&self.tseal.seal_tcp(&pt));
            self.tcp.write_all(&frame).expect("tcp write");
        }
    }
}

fn run(tcp: bool) -> anyhow::Result<Vec<f64>> {
    let mut phone = connect()?;
    let aus = access_units(CLIP);
    assert!(aus.len() >= 20 && aus[0].0, "clip should start with an idr");
    let sent: Arc<Mutex<HashMap<u32, Instant>>> = Arc::new(Mutex::new(HashMap::new()));
    let frames = Arc::clone(&phone.session.frames);
    let sent_c = Arc::clone(&sent);
    let n = aus.len();
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    let consumer = thread::spawn(move || -> anyhow::Result<Vec<f64>> {
        let mut dec = open(None)?;
        let _ = ready_tx.send(());
        let mut lat = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(10);
        while lat.len() < n && Instant::now() < deadline {
            let Some(cf) = frames.pop_wait(Duration::from_millis(50)) else {
                continue;
            };
            if dec.decode(&cf.data)?.is_some() {
                let t = sent_c.lock().expect("l")[&cf.frame_id];
                lat.push(t.elapsed().as_secs_f64() * 1000.0);
            }
        }
        Ok(lat)
    });
    let _ = ready_rx.recv_timeout(Duration::from_secs(5));
    let frame_gap = Duration::from_micros(16_667);
    for (i, (key, au)) in aus.iter().enumerate() {
        let id = i as u32 + 1;
        sent.lock().expect("l").insert(id, Instant::now());
        if tcp {
            phone.send_tcp(id, *key, au);
        } else {
            phone.send_udp(id, *key, au);
        }
        thread::sleep(frame_gap);
    }
    let lat = consumer.join().map_err(|_| anyhow::anyhow!("consumer"))??;
    phone.session.stop();
    Ok(lat)
}

fn report(name: &str, mut lat: Vec<f64>, total: usize) {
    assert!(!lat.is_empty(), "{name}: nothing decoded");
    lat.sort_by(|a, b| a.total_cmp(b));
    let p = |q: f64| lat[((lat.len() - 1) as f64 * q) as usize];
    println!(
        "{name}: {}/{total} frames, send-to-decoded p50 {:.2}ms p95 {:.2}ms max {:.2}ms",
        lat.len(),
        p(0.5),
        p(0.95),
        lat[lat.len() - 1]
    );
    assert!(lat.len() * 4 >= total * 3, "{name}: too many frames lost");
    assert!(p(0.95) < 100.0, "{name}: p95 too high");
}

#[test]
fn loopback_latency_udp_and_tcp() -> anyhow::Result<()> {
    if krapy_decode::available().is_empty() {
        return Ok(());
    }
    let total = access_units(CLIP).len();
    report("udp", run(false)?, total);
    report("tcp", run(true)?, total);
    Ok(())
}

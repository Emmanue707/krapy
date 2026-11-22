use std::fs;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream, UdpSocket};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use krapy_transport::crypto::{Opener, Sealer, SessionKeys, pair_secret};
use krapy_transport::reassembly::{FLAG_CODEC_CONFIG, FLAG_KEYFRAME};
use krapy_transport::wire::{
    Control, Datagram, Hello, encode_control, encode_datagram_body, encode_hello, encode_tcp_frame,
};
use rand::RngCore;

const CHUNK: usize = 1100;

struct PairUri {
    host: String,
    port: u16,
    k: [u8; 32],
}

fn parse_pair_uri(uri: &str) -> Result<PairUri> {
    let q = uri
        .strip_prefix("krapy://pair?")
        .or_else(|| uri.strip_prefix("uri: krapy://pair?"))
        .context("expected krapy://pair?...")?;
    let mut host = None;
    let mut port = None;
    let mut k = None;
    for part in q.split('&') {
        let Some((name, val)) = part.split_once('=') else {
            continue;
        };
        match name {
            "h" => host = Some(val.to_string()),
            "p" => port = Some(val.parse().context("port")?),
            "k" => {
                let bytes = URL_SAFE_NO_PAD.decode(val).context("pair key base64")?;
                if bytes.len() != 32 {
                    bail!("pair key must be 32 bytes");
                }
                let mut buf = [0u8; 32];
                buf.copy_from_slice(&bytes);
                k = Some(buf);
            }
            _ => {}
        }
    }
    Ok(PairUri {
        host: host.context("missing h")?,
        port: port.context("missing p")?,
        k: k.context("missing k")?,
    })
}

struct AccessUnit {
    flags: u8,
    data: Vec<u8>,
}

fn nal_header(nal: &[u8]) -> Option<u8> {
    let p = if nal.starts_with(&[0, 0, 0, 1]) {
        nal.get(4)
    } else if nal.starts_with(&[0, 0, 1]) {
        nal.get(3)
    } else {
        nal.first()
    }?;
    Some(*p & 0x1f)
}

fn scan_nals(data: &[u8]) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    let mut i = 0;
    while i + 3 < data.len() {
        let sc = if data[i..].starts_with(&[0, 0, 0, 1]) {
            i += 4;
            4
        } else if data[i..].starts_with(&[0, 0, 1]) {
            i += 3;
            3
        } else {
            i += 1;
            continue;
        };
        let start = i;
        while i + 3 < data.len() {
            if data[i..].starts_with(&[0, 0, 1]) || data[i..].starts_with(&[0, 0, 0, 1]) {
                break;
            }
            i += 1;
        }
        let end = i;
        if end > start {
            let mut nal = Vec::with_capacity(sc + (end - start));
            if sc == 4 {
                nal.extend_from_slice(&[0, 0, 0, 1]);
            } else {
                nal.extend_from_slice(&[0, 0, 1]);
            }
            nal.extend_from_slice(&data[start..end]);
            out.push(nal);
        }
    }
    out
}

fn annex_b_access_units(data: &[u8]) -> Vec<AccessUnit> {
    let nals = scan_nals(data);
    let mut sps_pps = Vec::new();
    let mut config_armed = false;
    let mut aus = Vec::new();
    for nal in nals {
        let Some(ty) = nal_header(&nal) else {
            continue;
        };
        if ty == 7 || ty == 8 {
            sps_pps.extend_from_slice(&nal);
            config_armed = true;
            continue;
        }
        if ty == 5 {
            if config_armed && !sps_pps.is_empty() {
                aus.push(AccessUnit {
                    flags: FLAG_CODEC_CONFIG,
                    data: sps_pps.clone(),
                });
                config_armed = false;
            }
            if let Some(last) = aus.last_mut()
                && last.flags & FLAG_KEYFRAME != 0
            {
                last.data.extend_from_slice(&nal);
                continue;
            }
            aus.push(AccessUnit {
                flags: FLAG_KEYFRAME,
                data: nal,
            });
        } else if ty == 1 {
            aus.push(AccessUnit {
                flags: 0,
                data: nal,
            });
        }
    }
    aus
}

fn fragments(data: &[u8], frame_id: u32, flags: u8, capture_us: u64) -> Vec<Datagram<'static>> {
    let count = data.len().div_ceil(CHUNK).max(1) as u16;
    (0..count)
        .map(|idx| {
            let start = idx as usize * CHUNK;
            let end = (start + CHUNK).min(data.len());
            let payload = if data.is_empty() {
                Vec::new()
            } else {
                data[start..end].to_vec()
            };
            Datagram::Video {
                frame_id,
                idx,
                count,
                flags,
                capture_us,
                data: std::borrow::Cow::Owned(payload),
            }
        })
        .collect()
}

fn send_udp(
    udp: &UdpSocket,
    dest: SocketAddr,
    sealer: &mut Sealer,
    d: &Datagram<'_>,
    loss: f64,
) -> Result<()> {
    if loss > 0.0 && rand::random::<f64>() < loss {
        return Ok(());
    }
    let body = encode_datagram_body(d).map_err(|e| anyhow::anyhow!("{e:?}"))?;
    let mut header = [0u8; 9];
    header[0] = body[0];
    header[1..9].copy_from_slice(&sealer.counter().to_be_bytes());
    let wire = sealer.seal_udp(&header, &body[1..]);
    udp.send_to(&wire, dest)?;
    Ok(())
}

struct Args {
    uri: String,
    clip: String,
    fps: u32,
    loss: f64,
    seconds: u64,
}

fn parse_args() -> Result<Args> {
    let mut raw: Vec<String> = std::env::args().skip(1).collect();
    if raw.is_empty() {
        bail!(
            "usage: fakephone '<krapy://pair uri>' <clip.h264> [--fps 60] [--loss 0.02] [--seconds 10]"
        );
    }
    let uri = raw.remove(0);
    if raw.is_empty() {
        bail!("missing clip path");
    }
    let clip = raw.remove(0);
    let mut fps = 60u32;
    let mut loss = 0.0f64;
    let mut seconds = 10u64;
    let mut i = 0;
    while i < raw.len() {
        match raw[i].as_str() {
            "--fps" => {
                i += 1;
                fps = raw.get(i).context("--fps")?.parse()?;
            }
            "--loss" => {
                i += 1;
                loss = raw.get(i).context("--loss")?.parse()?;
            }
            "--seconds" => {
                i += 1;
                seconds = raw.get(i).context("--seconds")?.parse()?;
            }
            other => bail!("unknown arg {other}"),
        }
        i += 1;
    }
    Ok(Args {
        uri,
        clip,
        fps,
        loss,
        seconds,
    })
}

fn main() -> Result<()> {
    let args = parse_args()?;
    let pair = parse_pair_uri(&args.uri)?;
    let clip = fs::read(&args.clip).with_context(|| format!("read {}", args.clip))?;
    let aus = annex_b_access_units(&clip);
    if aus.is_empty() {
        bail!("no access units in clip");
    }

    let dest = format!("{}:{}", pair.host, pair.port);
    let dest_addr: SocketAddr = dest.parse().context("desktop address")?;
    let mut tcp = TcpStream::connect(dest_addr).context("tcp connect")?;
    let udp = UdpSocket::bind("0.0.0.0:0").context("udp bind")?;
    udp.connect(dest_addr)?;

    let mut phone_id = [0u8; 8];
    rand::rng().fill_bytes(&mut phone_id);
    let mut nonce_p = [0u8; 16];
    rand::rng().fill_bytes(&mut nonce_p);
    let hello = Hello {
        phone_id,
        nonce_p,
        name: "fakephone".into(),
    };
    tcp.write_all(&encode_tcp_frame(
        &encode_hello(&hello).map_err(|e| anyhow::anyhow!("{e:?}"))?,
    ))?;
    let mut len = [0u8; 2];
    tcp.read_exact(&mut len)?;
    let l = u16::from_be_bytes(len) as usize;
    let mut body = vec![0u8; l];
    tcp.read_exact(&mut body)?;
    let welcome =
        krapy_transport::wire::decode_welcome(&body).map_err(|e| anyhow::anyhow!("{e:?}"))?;
    let s = pair_secret(&pair.k);
    let keys = SessionKeys::derive(&s, &nonce_p, &welcome.nonce_d);

    let start = encode_control(&Control::Start {
        width: 1280,
        height: 720,
        fps: args.fps.min(255) as u8,
        bitrate: 4_000_000,
        codec: 1,
    })
    .map_err(|e| anyhow::anyhow!("{e:?}"))?;
    let mut tcp_seal = Sealer::new(&keys.tcp_p2d);
    tcp.write_all(&encode_tcp_frame(&tcp_seal.seal_tcp(&start)))?;

    let keyframe_jump = Arc::new(AtomicBool::new(false));
    let frames_sent = Arc::new(AtomicU64::new(0));
    let quality_log: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));

    let kj = Arc::clone(&keyframe_jump);
    let ql = Arc::clone(&quality_log);
    let tcp_read = tcp.try_clone().context("tcp clone")?;
    let read_keys = keys.tcp_d2p;
    thread::spawn(move || {
        let mut opener = Opener::new(&read_keys);
        let mut s = tcp_read;
        s.set_read_timeout(Some(Duration::from_millis(200))).ok();
        let mut len = [0u8; 2];
        loop {
            match s.read_exact(&mut len) {
                Ok(()) => {}
                Err(e)
                    if e.kind() == std::io::ErrorKind::WouldBlock
                        || e.kind() == std::io::ErrorKind::TimedOut =>
                {
                    continue;
                }
                Err(_) => break,
            }
            let l = u16::from_be_bytes(len) as usize;
            let mut wire = vec![0u8; l];
            if s.read_exact(&mut wire).is_err() {
                break;
            }
            let Ok(pt) = opener.open_tcp(&wire) else {
                continue;
            };
            let Ok(msg) = krapy_transport::wire::decode_control(&pt) else {
                continue;
            };
            match msg {
                Control::Quality {
                    bitrate,
                    fps,
                    max_size,
                } => {
                    if let Ok(mut g) = ql.lock() {
                        g.push(format!(
                            "quality bitrate={bitrate} fps={fps} max_size={max_size}"
                        ));
                    }
                }
                Control::Keyframe => kj.store(true, Ordering::Relaxed),
                _ => {}
            }
        }
    });

    let ping_in = keys.udp_d2p;
    let useal = Arc::new(Mutex::new(Sealer::new(&keys.udp_p2d)));
    let pseal = Arc::clone(&useal);
    let udp_read = udp.try_clone().context("udp clone")?;
    thread::spawn(move || {
        let mut opener = Opener::new(&ping_in);
        let mut buf = [0u8; 2048];
        udp_read
            .set_read_timeout(Some(Duration::from_millis(100)))
            .ok();
        loop {
            let Ok((n, _)) = udp_read.recv_from(&mut buf) else {
                continue;
            };
            if n < 9 + 16 {
                continue;
            }
            let Ok(pt) = opener.open_udp(&buf[..n]) else {
                continue;
            };
            let mut full = Vec::with_capacity(1 + pt.len());
            full.push(buf[0]);
            full.extend_from_slice(&pt);
            let Ok(dg) = krapy_transport::wire::decode_datagram_body(&full) else {
                continue;
            };
            if let Datagram::Ping { id, t_us } = dg {
                let pong = Datagram::Pong { id, t_us };
                let body = match encode_datagram_body(&pong) {
                    Ok(b) => b,
                    Err(_) => continue,
                };
                let Ok(mut seal) = pseal.lock() else {
                    break;
                };
                let mut header = [0u8; 9];
                header[0] = body[0];
                header[1..9].copy_from_slice(&seal.counter().to_be_bytes());
                let wire = seal.seal_udp(&header, &body[1..]);
                drop(seal);
                let _ = udp_read.send(&wire);
            }
        }
    });

    send_udp(
        &udp,
        dest_addr,
        &mut useal.lock().expect("sealer"),
        &Datagram::Keepalive,
        0.0,
    )?;

    let frame_interval = Duration::from_secs_f64(1.0 / args.fps as f64);
    let end = Instant::now() + Duration::from_secs(args.seconds);
    let mut frame_id = 1u32;
    let mut au_idx = 0usize;
    let capture_base = Instant::now();

    while Instant::now() < end {
        if keyframe_jump.swap(false, Ordering::Relaxed) {
            let mut j = au_idx;
            while j < aus.len() {
                if aus[j].flags & FLAG_KEYFRAME != 0 {
                    au_idx = j;
                    break;
                }
                j += 1;
            }
            if j >= aus.len()
                && let Some(i) = aus.iter().position(|a| a.flags & FLAG_KEYFRAME != 0)
            {
                au_idx = i;
            }
        }

        let au = &aus[au_idx];
        let capture_us = capture_base.elapsed().as_micros() as u64;
        for dg in fragments(&au.data, frame_id, au.flags, capture_us) {
            send_udp(
                &udp,
                dest_addr,
                &mut useal.lock().expect("sealer"),
                &dg,
                args.loss,
            )?;
        }
        frames_sent.fetch_add(1, Ordering::Relaxed);
        frame_id = frame_id.wrapping_add(1);
        au_idx += 1;
        if au_idx >= aus.len() {
            au_idx = 0;
        }

        thread::sleep(frame_interval);
        if Instant::now().duration_since(capture_base) >= Duration::from_secs(1) {
            send_udp(
                &udp,
                dest_addr,
                &mut useal.lock().expect("sealer"),
                &Datagram::Keepalive,
                0.0,
            )?;
        }
    }

    println!(
        "fakephone frames_sent={} quality_changes={}",
        frames_sent.load(Ordering::Relaxed),
        quality_log.lock().map(|g| g.len()).unwrap_or(0)
    );
    if let Ok(g) = quality_log.lock() {
        for line in g.iter() {
            println!("{line}");
        }
    }
    Ok(())
}

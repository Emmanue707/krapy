use std::borrow::Cow;
use std::io::{Read, Write};
use std::net::{TcpStream, UdpSocket};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use krapy_decode::{Decoder, open};
use krapy_transport::crypto::{Sealer, SessionKeys, pair_secret};
use krapy_transport::reassembly::FLAG_KEYFRAME;
use krapy_transport::wire::{
    Control, Datagram, Hello, encode_control, encode_datagram_body, encode_hello, encode_tcp_frame,
};
use krapy_transport::{AcceptMode, Listener, Session};

const CLIP: &[u8] = include_bytes!("../../krapy-decode/tests/data/clip.h264");

fn pick_port() -> u16 {
    UdpSocket::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn fragments(data: &[u8], frame_id: u32) -> Vec<Datagram<'static>> {
    const CHUNK: usize = 1100;
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
                flags: FLAG_KEYFRAME,
                capture_us: 1_000_000,
                data: Cow::Owned(payload),
            }
        })
        .collect()
}

fn send_phone(udp: &UdpSocket, dest: &str, sealer: &mut Sealer, d: &Datagram<'_>) {
    let body = encode_datagram_body(d).unwrap();
    let mut header = [0u8; 9];
    header[0] = body[0];
    header[1..9].copy_from_slice(&sealer.counter().to_be_bytes());
    let wire = sealer.seal_udp(&header, &body[1..]);
    udp.send_to(&wire, dest).unwrap();
}

#[test]
fn fake_phone_pair_decode_no_window() -> anyhow::Result<()> {
    if krapy_decode::available().is_empty() {
        return Ok(());
    }
    let port = pick_port();
    let desktop_id = [0xde, 0xad, 0xbe, 0xef, 0, 0, 0, 1];
    let listener = Listener::bind(port, desktop_id)?;
    let phone_id = [0x11u8; 8];
    let k = [0x22u8; 32];
    let paired = Arc::new(Mutex::new(None));
    let paired_c = Arc::clone(&paired);
    let accept_h = thread::spawn(move || {
        let mut cb = |pid: [u8; 8], s: [u8; 32]| {
            *paired_c.lock().unwrap() = Some((pid, s));
        };
        listener.accept(AcceptMode::Pair {
            k,
            on_paired: &mut cb,
        })
    });

    thread::sleep(Duration::from_millis(20));
    let dest = format!("127.0.0.1:{port}");
    let mut tcp = TcpStream::connect(&dest)?;
    let udp = UdpSocket::bind("127.0.0.1:0")?;
    let mut nonce_p = [0u8; 16];
    rand::Rng::fill(&mut rand::rng(), &mut nonce_p);
    let hello = Hello {
        phone_id,
        nonce_p,
        name: "test-phone".into(),
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
    let s = pair_secret(&k);
    let keys = SessionKeys::derive(&s, &nonce_p, &welcome.nonce_d);
    let start = encode_control(&Control::Start {
        width: 160,
        height: 96,
        fps: 30,
        bitrate: 1_000_000,
        codec: 1,
    })
    .map_err(|e| anyhow::anyhow!("{e:?}"))?;
    let mut tseal = Sealer::new(&keys.tcp_p2d);
    tcp.write_all(&encode_tcp_frame(&tseal.seal_tcp(&start)))?;

    let session: Session = accept_h
        .join()
        .unwrap()
        .map_err(|e| anyhow::anyhow!("{e:?}"))?;
    assert!(paired.lock().unwrap().is_some());

    let mut useal = Sealer::new(&keys.udp_p2d);
    send_phone(&udp, &dest, &mut useal, &Datagram::Keepalive);
    for dg in fragments(CLIP, 1) {
        send_phone(&udp, &dest, &mut useal, &dg);
    }

    let mut annexb = None;
    for _ in 0..100 {
        if let Some(f) = session.frames.pop() {
            annexb = Some(f.data);
            break;
        }
        thread::sleep(Duration::from_millis(20));
    }
    let annexb = annexb.expect("reassembled frame");
    let mut dec: Box<dyn Decoder> = open(None)?;
    let frame = dec.decode(&annexb)?.expect("decoded frame");
    assert_eq!(frame.width, 160);
    assert_eq!(frame.height, 96);
    session.stop();
    Ok(())
}

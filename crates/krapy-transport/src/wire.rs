use bytes::Bytes;
use std::borrow::Cow;

const MAGIC: &[u8; 4] = b"KRPY";
const VER: u8 = 1;
const MAX_VIDEO_DATA: usize = 1100;
const MAX_NAME: usize = 255;
const MAX_UTF8: usize = 64 * 1024;
pub const MAX_TCP_VIDEO: usize = 32 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hello {
    pub phone_id: [u8; 8],
    pub nonce_p: [u8; 16],
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Welcome {
    pub desktop_id: [u8; 8],
    pub nonce_d: [u8; 16],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Control {
    Start {
        width: u16,
        height: u16,
        fps: u8,
        bitrate: u32,
        codec: u8,
    },
    Quality {
        bitrate: u32,
        fps: u8,
        max_size: u16,
    },
    Keyframe,
    Format {
        width: u16,
        height: u16,
    },
    Clipboard {
        text: String,
    },
    Touch {
        action: u8,
        pointer: u8,
        x: u16,
        y: u16,
    },
    Key {
        code: u8,
    },
    Text {
        text: String,
    },
    // video over tcp, used when udp can't get through (usb / adb reverse)
    VideoChunk {
        frame_id: u32,
        idx: u16,
        count: u16,
        flags: u8,
        capture_us: u64,
        data: Vec<u8>,
    },
    AudioChunk {
        seq: u32,
        pts_us: u64,
        data: Vec<u8>,
    },
    PhoneStats {
        enc_avg_us: u32,
        enc_max_us: u32,
        fps: u8,
        bitrate: u32,
    },
    DesktopStats {
        fps: u8,
        latency_ms: u16,
        decode_ms: u16,
        loss_permille: u16,
    },
    PresetRequest {
        preset: u8,
    },
    Paused {
        on: u8,
    },
    RotateView,
    AudioState {
        on: u8,
    },
    Bye,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Datagram<'a> {
    Video {
        frame_id: u32,
        idx: u16,
        count: u16,
        flags: u8,
        capture_us: u64,
        data: Cow<'a, [u8]>,
    },
    Ping {
        id: u32,
        t_us: u64,
    },
    Pong {
        id: u32,
        t_us: u64,
        phone_us: Option<u64>,
    },
    Audio {
        seq: u32,
        pts_us: u64,
        data: Cow<'a, [u8]>,
    },
    Keepalive,
}

pub const DG_VIDEO: u8 = 0x01;
pub const DG_PING: u8 = 0x02;
pub const DG_PONG: u8 = 0x03;
pub const DG_KEEPALIVE: u8 = 0x05;
pub const DG_AUDIO: u8 = 0x06;
pub const MAX_AUDIO_DATA: usize = 960;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WireError {
    Truncated,
    BadMagic,
    BadVersion,
    BadUtf8,
    BadLength,
    UnknownType,
}

pub fn encode_tcp_frame(body: &[u8]) -> Vec<u8> {
    let len = body.len();
    assert!(len <= u16::MAX as usize);
    let mut out = Vec::with_capacity(2 + len);
    out.extend_from_slice(&(len as u16).to_be_bytes());
    out.extend_from_slice(body);
    out
}

pub fn decode_tcp_frame(buf: &[u8]) -> Result<&[u8], WireError> {
    if buf.len() < 2 {
        return Err(WireError::Truncated);
    }
    let len = u16::from_be_bytes([buf[0], buf[1]]) as usize;
    if buf.len() < 2 + len {
        return Err(WireError::Truncated);
    }
    Ok(&buf[2..2 + len])
}

pub fn encode_hello(h: &Hello) -> Result<Vec<u8>, WireError> {
    if h.name.len() > MAX_NAME {
        return Err(WireError::BadLength);
    }
    let mut out = Vec::with_capacity(4 + 1 + 8 + 16 + 1 + h.name.len());
    out.extend_from_slice(MAGIC);
    out.push(VER);
    out.extend_from_slice(&h.phone_id);
    out.extend_from_slice(&h.nonce_p);
    out.push(h.name.len() as u8);
    out.extend_from_slice(h.name.as_bytes());
    Ok(out)
}

pub fn decode_hello(buf: &[u8]) -> Result<Hello, WireError> {
    let min = 4 + 1 + 8 + 16 + 1;
    if buf.len() < min {
        return Err(WireError::Truncated);
    }
    if &buf[0..4] != MAGIC {
        return Err(WireError::BadMagic);
    }
    if buf[4] != VER {
        return Err(WireError::BadVersion);
    }
    let mut phone_id = [0u8; 8];
    phone_id.copy_from_slice(&buf[5..13]);
    let mut nonce_p = [0u8; 16];
    nonce_p.copy_from_slice(&buf[13..29]);
    let name_len = buf[29] as usize;
    if buf.len() < min + name_len {
        return Err(WireError::Truncated);
    }
    let name = std::str::from_utf8(&buf[30..30 + name_len])
        .map_err(|_| WireError::BadUtf8)?
        .to_string();
    Ok(Hello {
        phone_id,
        nonce_p,
        name,
    })
}

pub fn encode_welcome(w: &Welcome) -> Vec<u8> {
    let mut out = Vec::with_capacity(4 + 1 + 8 + 16);
    out.extend_from_slice(MAGIC);
    out.push(VER);
    out.extend_from_slice(&w.desktop_id);
    out.extend_from_slice(&w.nonce_d);
    out
}

pub fn decode_welcome(buf: &[u8]) -> Result<Welcome, WireError> {
    if buf.len() < 4 + 1 + 8 + 16 {
        return Err(WireError::Truncated);
    }
    if &buf[0..4] != MAGIC {
        return Err(WireError::BadMagic);
    }
    if buf[4] != VER {
        return Err(WireError::BadVersion);
    }
    let mut desktop_id = [0u8; 8];
    desktop_id.copy_from_slice(&buf[5..13]);
    let mut nonce_d = [0u8; 16];
    nonce_d.copy_from_slice(&buf[13..29]);
    Ok(Welcome {
        desktop_id,
        nonce_d,
    })
}

pub fn encode_control(c: &Control) -> Result<Vec<u8>, WireError> {
    let mut out = Vec::new();
    match c {
        Control::Start {
            width,
            height,
            fps,
            bitrate,
            codec,
        } => {
            out.push(0x01);
            out.extend_from_slice(&width.to_be_bytes());
            out.extend_from_slice(&height.to_be_bytes());
            out.push(*fps);
            out.extend_from_slice(&bitrate.to_be_bytes());
            out.push(*codec);
        }
        Control::Quality {
            bitrate,
            fps,
            max_size,
        } => {
            out.push(0x02);
            out.extend_from_slice(&bitrate.to_be_bytes());
            out.push(*fps);
            out.extend_from_slice(&max_size.to_be_bytes());
        }
        Control::Keyframe => out.push(0x03),
        Control::Format { width, height } => {
            out.push(0x04);
            out.extend_from_slice(&width.to_be_bytes());
            out.extend_from_slice(&height.to_be_bytes());
        }
        Control::Clipboard { text } => {
            if text.len() > MAX_UTF8 {
                return Err(WireError::BadLength);
            }
            out.push(0x10);
            out.extend_from_slice(text.as_bytes());
        }
        Control::Touch {
            action,
            pointer,
            x,
            y,
        } => {
            out.push(0x20);
            out.push(*action);
            out.push(*pointer);
            out.extend_from_slice(&x.to_be_bytes());
            out.extend_from_slice(&y.to_be_bytes());
        }
        Control::Key { code } => {
            out.push(0x21);
            out.push(*code);
        }
        Control::Text { text } => {
            if text.len() > MAX_UTF8 {
                return Err(WireError::BadLength);
            }
            out.push(0x22);
            out.extend_from_slice(text.as_bytes());
        }
        Control::VideoChunk {
            frame_id,
            idx,
            count,
            flags,
            capture_us,
            data,
        } => {
            if data.len() > MAX_TCP_VIDEO {
                return Err(WireError::BadLength);
            }
            out.push(0x30);
            out.extend_from_slice(&frame_id.to_be_bytes());
            out.extend_from_slice(&idx.to_be_bytes());
            out.extend_from_slice(&count.to_be_bytes());
            out.push(*flags);
            out.extend_from_slice(&capture_us.to_be_bytes());
            out.extend_from_slice(data);
        }
        Control::AudioChunk { seq, pts_us, data } => {
            if data.len() > MAX_AUDIO_DATA {
                return Err(WireError::BadLength);
            }
            out.push(0x31);
            out.extend_from_slice(&seq.to_be_bytes());
            out.extend_from_slice(&pts_us.to_be_bytes());
            out.extend_from_slice(data);
        }
        Control::PhoneStats {
            enc_avg_us,
            enc_max_us,
            fps,
            bitrate,
        } => {
            out.push(0x40);
            out.extend_from_slice(&enc_avg_us.to_be_bytes());
            out.extend_from_slice(&enc_max_us.to_be_bytes());
            out.push(*fps);
            out.extend_from_slice(&bitrate.to_be_bytes());
        }
        Control::DesktopStats {
            fps,
            latency_ms,
            decode_ms,
            loss_permille,
        } => {
            out.push(0x41);
            out.push(*fps);
            out.extend_from_slice(&latency_ms.to_be_bytes());
            out.extend_from_slice(&decode_ms.to_be_bytes());
            out.extend_from_slice(&loss_permille.to_be_bytes());
        }
        Control::PresetRequest { preset } => {
            out.push(0x42);
            out.push(*preset);
        }
        Control::Paused { on } => {
            out.push(0x43);
            out.push(*on);
        }
        Control::RotateView => out.push(0x45),
        Control::AudioState { on } => {
            out.push(0x46);
            out.push(*on);
        }
        Control::Bye => out.push(0x7f),
    }
    Ok(out)
}

pub fn decode_control(buf: &[u8]) -> Result<Control, WireError> {
    if buf.is_empty() {
        return Err(WireError::Truncated);
    }
    match buf[0] {
        0x01 => {
            if buf.len() < 1 + 2 + 2 + 1 + 4 + 1 {
                return Err(WireError::Truncated);
            }
            Ok(Control::Start {
                width: u16::from_be_bytes(buf[1..3].try_into().expect("len")),
                height: u16::from_be_bytes(buf[3..5].try_into().expect("len")),
                fps: buf[5],
                bitrate: u32::from_be_bytes(buf[6..10].try_into().expect("len")),
                codec: buf[10],
            })
        }
        0x02 => {
            if buf.len() < 1 + 4 + 1 + 2 {
                return Err(WireError::Truncated);
            }
            Ok(Control::Quality {
                bitrate: u32::from_be_bytes(buf[1..5].try_into().expect("len")),
                fps: buf[5],
                max_size: u16::from_be_bytes(buf[6..8].try_into().expect("len")),
            })
        }
        0x03 => Ok(Control::Keyframe),
        0x04 => {
            if buf.len() < 1 + 4 {
                return Err(WireError::Truncated);
            }
            Ok(Control::Format {
                width: u16::from_be_bytes(buf[1..3].try_into().expect("len")),
                height: u16::from_be_bytes(buf[3..5].try_into().expect("len")),
            })
        }
        0x10 => {
            let text = std::str::from_utf8(&buf[1..])
                .map_err(|_| WireError::BadUtf8)?
                .to_string();
            Ok(Control::Clipboard { text })
        }
        0x20 => {
            if buf.len() < 1 + 6 {
                return Err(WireError::Truncated);
            }
            Ok(Control::Touch {
                action: buf[1],
                pointer: buf[2],
                x: u16::from_be_bytes(buf[3..5].try_into().expect("len")),
                y: u16::from_be_bytes(buf[5..7].try_into().expect("len")),
            })
        }
        0x21 => {
            if buf.len() < 2 {
                return Err(WireError::Truncated);
            }
            Ok(Control::Key { code: buf[1] })
        }
        0x22 => {
            let text = std::str::from_utf8(&buf[1..])
                .map_err(|_| WireError::BadUtf8)?
                .to_string();
            Ok(Control::Text { text })
        }
        0x30 => {
            let hdr = 1 + 4 + 2 + 2 + 1 + 8;
            if buf.len() < hdr {
                return Err(WireError::Truncated);
            }
            if buf.len() - hdr > MAX_TCP_VIDEO {
                return Err(WireError::BadLength);
            }
            Ok(Control::VideoChunk {
                frame_id: u32::from_be_bytes(buf[1..5].try_into().expect("len")),
                idx: u16::from_be_bytes(buf[5..7].try_into().expect("len")),
                count: u16::from_be_bytes(buf[7..9].try_into().expect("len")),
                flags: buf[9],
                capture_us: u64::from_be_bytes(buf[10..18].try_into().expect("len")),
                data: buf[hdr..].to_vec(),
            })
        }
        0x31 => {
            let hdr = 1 + 4 + 8;
            if buf.len() < hdr {
                return Err(WireError::Truncated);
            }
            if buf.len() - hdr > MAX_AUDIO_DATA {
                return Err(WireError::BadLength);
            }
            Ok(Control::AudioChunk {
                seq: u32::from_be_bytes(buf[1..5].try_into().expect("len")),
                pts_us: u64::from_be_bytes(buf[5..13].try_into().expect("len")),
                data: buf[hdr..].to_vec(),
            })
        }
        0x40 => {
            if buf.len() < 1 + 4 + 4 + 1 + 4 {
                return Err(WireError::Truncated);
            }
            Ok(Control::PhoneStats {
                enc_avg_us: u32::from_be_bytes(buf[1..5].try_into().expect("len")),
                enc_max_us: u32::from_be_bytes(buf[5..9].try_into().expect("len")),
                fps: buf[9],
                bitrate: u32::from_be_bytes(buf[10..14].try_into().expect("len")),
            })
        }
        0x41 => {
            if buf.len() < 1 + 1 + 2 + 2 + 2 {
                return Err(WireError::Truncated);
            }
            Ok(Control::DesktopStats {
                fps: buf[1],
                latency_ms: u16::from_be_bytes(buf[2..4].try_into().expect("len")),
                decode_ms: u16::from_be_bytes(buf[4..6].try_into().expect("len")),
                loss_permille: u16::from_be_bytes(buf[6..8].try_into().expect("len")),
            })
        }
        0x42 => {
            if buf.len() < 2 {
                return Err(WireError::Truncated);
            }
            Ok(Control::PresetRequest { preset: buf[1] })
        }
        0x43 => {
            if buf.len() < 2 {
                return Err(WireError::Truncated);
            }
            Ok(Control::Paused { on: buf[1] })
        }
        0x45 => Ok(Control::RotateView),
        0x46 => {
            if buf.len() < 2 {
                return Err(WireError::Truncated);
            }
            Ok(Control::AudioState { on: buf[1] })
        }
        0x7f => Ok(Control::Bye),
        _ => Err(WireError::UnknownType),
    }
}

pub fn encode_datagram_body(d: &Datagram<'_>) -> Result<Vec<u8>, WireError> {
    let mut out = Vec::new();
    match d {
        Datagram::Video {
            frame_id,
            idx,
            count,
            flags,
            capture_us,
            data,
        } => {
            if data.len() > MAX_VIDEO_DATA {
                return Err(WireError::BadLength);
            }
            out.push(DG_VIDEO);
            out.extend_from_slice(&frame_id.to_be_bytes());
            out.extend_from_slice(&idx.to_be_bytes());
            out.extend_from_slice(&count.to_be_bytes());
            out.push(*flags);
            out.extend_from_slice(&capture_us.to_be_bytes());
            out.extend_from_slice(data);
        }
        Datagram::Ping { id, t_us } => {
            out.push(DG_PING);
            out.extend_from_slice(&id.to_be_bytes());
            out.extend_from_slice(&t_us.to_be_bytes());
        }
        Datagram::Pong { id, t_us, phone_us } => {
            out.push(DG_PONG);
            out.extend_from_slice(&id.to_be_bytes());
            out.extend_from_slice(&t_us.to_be_bytes());
            if let Some(pu) = phone_us {
                out.extend_from_slice(&pu.to_be_bytes());
            }
        }
        Datagram::Audio { seq, pts_us, data } => {
            if data.len() > MAX_AUDIO_DATA {
                return Err(WireError::BadLength);
            }
            out.push(DG_AUDIO);
            out.extend_from_slice(&seq.to_be_bytes());
            out.extend_from_slice(&pts_us.to_be_bytes());
            out.extend_from_slice(data);
        }
        Datagram::Keepalive => out.push(DG_KEEPALIVE),
    }
    Ok(out)
}

pub fn decode_datagram_body(buf: &[u8]) -> Result<Datagram<'_>, WireError> {
    if buf.is_empty() {
        return Err(WireError::Truncated);
    }
    match buf[0] {
        DG_VIDEO => {
            let hdr = 1 + 4 + 2 + 2 + 1 + 8;
            if buf.len() < hdr {
                return Err(WireError::Truncated);
            }
            let data_len = buf.len() - hdr;
            if data_len > MAX_VIDEO_DATA {
                return Err(WireError::BadLength);
            }
            Ok(Datagram::Video {
                frame_id: u32::from_be_bytes(buf[1..5].try_into().expect("len")),
                idx: u16::from_be_bytes(buf[5..7].try_into().expect("len")),
                count: u16::from_be_bytes(buf[7..9].try_into().expect("len")),
                flags: buf[9],
                capture_us: u64::from_be_bytes(buf[10..18].try_into().expect("len")),
                data: Cow::Borrowed(&buf[18..]),
            })
        }
        DG_PING => {
            if buf.len() < 1 + 12 {
                return Err(WireError::Truncated);
            }
            Ok(Datagram::Ping {
                id: u32::from_be_bytes(buf[1..5].try_into().expect("len")),
                t_us: u64::from_be_bytes(buf[5..13].try_into().expect("len")),
            })
        }
        DG_PONG => {
            if buf.len() < 1 + 12 {
                return Err(WireError::Truncated);
            }
            let phone_us = if buf.len() > 20 {
                Some(u64::from_be_bytes(buf[13..21].try_into().expect("len")))
            } else {
                None
            };
            Ok(Datagram::Pong {
                id: u32::from_be_bytes(buf[1..5].try_into().expect("len")),
                t_us: u64::from_be_bytes(buf[5..13].try_into().expect("len")),
                phone_us,
            })
        }
        DG_AUDIO => {
            let hdr = 1 + 4 + 8;
            if buf.len() < hdr {
                return Err(WireError::Truncated);
            }
            let data_len = buf.len() - hdr;
            if data_len > MAX_AUDIO_DATA {
                return Err(WireError::BadLength);
            }
            Ok(Datagram::Audio {
                seq: u32::from_be_bytes(buf[1..5].try_into().expect("len")),
                pts_us: u64::from_be_bytes(buf[5..13].try_into().expect("len")),
                data: Cow::Borrowed(&buf[13..]),
            })
        }
        DG_KEEPALIVE => {
            if buf.len() != 1 {
                return Err(WireError::BadLength);
            }
            Ok(Datagram::Keepalive)
        }
        _ => Err(WireError::UnknownType),
    }
}

pub fn video_from_bytes(
    frame_id: u32,
    idx: u16,
    count: u16,
    flags: u8,
    capture_us: u64,
    data: Bytes,
) -> Datagram<'static> {
    Datagram::Video {
        frame_id,
        idx,
        count,
        flags,
        capture_us,
        data: Cow::Owned(data.to_vec()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip_hello() {
        let h = Hello {
            phone_id: [1u8; 8],
            nonce_p: [2u8; 16],
            name: "phone".into(),
        };
        let enc = encode_hello(&h).expect("enc");
        assert_eq!(decode_hello(&enc).expect("dec"), h);
    }

    #[test]
    fn hello_roundtrip_and_truncated() {
        roundtrip_hello();
        assert_eq!(decode_hello(b"KRPY"), Err(WireError::Truncated));
        let mut enc = encode_hello(&Hello {
            phone_id: [0; 8],
            nonce_p: [0; 16],
            name: "x".into(),
        })
        .expect("e");
        enc.truncate(10);
        assert_eq!(decode_hello(&enc), Err(WireError::Truncated));
    }

    #[test]
    fn welcome_roundtrip() {
        let w = Welcome {
            desktop_id: [9u8; 8],
            nonce_d: [8u8; 16],
        };
        let enc = encode_welcome(&w);
        assert_eq!(decode_welcome(&enc).expect("dec"), w);
        assert_eq!(decode_welcome(&enc[..20]), Err(WireError::Truncated));
    }

    #[test]
    fn all_controls_roundtrip() {
        let msgs = [
            Control::Start {
                width: 1920,
                height: 1080,
                fps: 60,
                bitrate: 4_000_000,
                codec: 1,
            },
            Control::Quality {
                bitrate: 2_000_000,
                fps: 30,
                max_size: 1280,
            },
            Control::Keyframe,
            Control::Format {
                width: 720,
                height: 1280,
            },
            Control::Clipboard { text: "hi".into() },
            Control::Touch {
                action: 1,
                pointer: 0,
                x: 100,
                y: 200,
            },
            Control::Key { code: 2 },
            Control::Text {
                text: "type".into(),
            },
            Control::VideoChunk {
                frame_id: 7,
                idx: 1,
                count: 3,
                flags: 1,
                capture_us: 123_456,
                data: vec![0, 0, 0, 1, 0x65],
            },
            Control::AudioChunk {
                seq: 1,
                pts_us: 50_000,
                data: vec![0; 960],
            },
            Control::PhoneStats {
                enc_avg_us: 5000,
                enc_max_us: 12000,
                fps: 60,
                bitrate: 8_000_000,
            },
            Control::DesktopStats {
                fps: 60,
                latency_ms: 14,
                decode_ms: 3,
                loss_permille: 5,
            },
            Control::PresetRequest { preset: 1 },
            Control::Paused { on: 1 },
            Control::RotateView,
            Control::AudioState { on: 0 },
            Control::Bye,
        ];
        for m in msgs {
            let enc = encode_control(&m).expect("enc");
            let dec = decode_control(&enc).expect("dec");
            assert_eq!(dec, m);
        }
        assert_eq!(decode_control(&[0x01]), Err(WireError::Truncated));
    }

    #[test]
    fn datagram_roundtrips() {
        let v = Datagram::Video {
            frame_id: 7,
            idx: 0,
            count: 2,
            flags: 1,
            capture_us: 1_000_000,
            data: Cow::Borrowed(b"annexb"),
        };
        let enc = encode_datagram_body(&v).expect("enc");
        match decode_datagram_body(&enc).expect("dec") {
            Datagram::Video { data, .. } => assert_eq!(&data[..], b"annexb"),
            _ => panic!("typ"),
        }
        let p = Datagram::Ping { id: 1, t_us: 99 };
        let enc = encode_datagram_body(&p).expect("enc");
        assert_eq!(decode_datagram_body(&enc).expect("d"), p);
        let pong = Datagram::Pong {
            id: 1,
            t_us: 99,
            phone_us: None,
        };
        let enc = encode_datagram_body(&pong).expect("enc");
        assert_eq!(decode_datagram_body(&enc).expect("d"), pong);
        let pong2 = Datagram::Pong {
            id: 2,
            t_us: 100,
            phone_us: Some(9_999_999),
        };
        let enc2 = encode_datagram_body(&pong2).expect("enc");
        assert_eq!(decode_datagram_body(&enc2).expect("d"), pong2);
        let aud = Datagram::Audio {
            seq: 3,
            pts_us: 1,
            data: Cow::Borrowed(&[1, 2]),
        };
        let enca = encode_datagram_body(&aud).expect("enc");
        match decode_datagram_body(&enca).expect("d") {
            Datagram::Audio { data, .. } => assert_eq!(&data[..], &[1, 2]),
            _ => panic!("typ"),
        }
        let k = Datagram::Keepalive;
        let enc = encode_datagram_body(&k).expect("enc");
        assert_eq!(decode_datagram_body(&enc).expect("d"), k);
        assert_eq!(decode_datagram_body(&[DG_VIDEO]), Err(WireError::Truncated));
    }

    #[test]
    fn tcp_frame_roundtrip() {
        let body = encode_hello(&Hello {
            phone_id: [0; 8],
            nonce_p: [0; 16],
            name: "a".into(),
        })
        .expect("h");
        let frame = encode_tcp_frame(&body);
        let dec = decode_tcp_frame(&frame).expect("d");
        assert_eq!(dec, body);
        assert_eq!(decode_tcp_frame(&frame[..1]), Err(WireError::Truncated));
    }
}

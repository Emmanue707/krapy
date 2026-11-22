use krapy_decode::open;

const CLIP: &[u8] = include_bytes!("data/clip.h264");
const FLAG_CODEC_CONFIG: u8 = 2;
const FLAG_KEYFRAME: u8 = 1;

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

fn decode_stream(dec_name: &str) -> anyhow::Result<()> {
    let mut dec = open(Some(dec_name))?;
    let aus = annex_b_access_units(CLIP);
    let mut past_idr = false;
    let mut pictures = 0usize;
    let video_aus = aus
        .iter()
        .filter(|a| a.flags & FLAG_CODEC_CONFIG == 0)
        .count();
    for (idx, au) in aus.into_iter().enumerate() {
        if au.flags & FLAG_CODEC_CONFIG != 0 {
            let r = dec
                .decode(&au.data)
                .map_err(|e| anyhow::anyhow!("{dec_name} au {idx} config: {e}"))?;
            assert!(r.is_none(), "{dec_name} config au should not yield frame");
            continue;
        }
        if !past_idr {
            assert!(au.flags & FLAG_KEYFRAME != 0);
            past_idr = true;
        }
        let r = dec
            .decode(&au.data)
            .map_err(|e| anyhow::anyhow!("{dec_name} au {idx} flags={}: {e}", au.flags))?;
        if let Some(frame) = r {
            assert!(frame.width > 0 && frame.height > 0, "{dec_name} dimensions");
            pictures += 1;
        }
    }
    assert!(
        pictures >= video_aus.saturating_sub(1),
        "{dec_name} got {pictures} pictures from {video_aus} access units"
    );
    if dec_name == "ffmpeg" {
        assert_eq!(pictures, video_aus, "ffmpeg should picture every video au");
    }
    Ok(())
}

#[test]
#[cfg(feature = "openh264")]
fn openh264_streaming_access_units() -> anyhow::Result<()> {
    decode_stream("openh264")
}

#[test]
#[cfg(feature = "ffmpeg")]
fn ffmpeg_streaming_access_units() -> anyhow::Result<()> {
    decode_stream("ffmpeg")
}

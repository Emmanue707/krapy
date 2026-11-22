use krapy_decode::{open, Decoder};

const CLIP: &[u8] = include_bytes!("data/clip.h264");
const W: u32 = 160;
const H: u32 = 96;

fn first_frame(mut dec: Box<dyn Decoder>) -> anyhow::Result<Option<krapy_decode::Frame>> {
    if let Some(f) = dec.decode(CLIP)? {
        return Ok(Some(f));
    }
    dec.flush()
}

fn assert_frame(dec_name: &str, frame: krapy_decode::Frame) {
    assert_eq!(frame.width, W, "{dec_name} width");
    assert_eq!(frame.height, H, "{dec_name} height");
    assert!(frame.y.len() >= (W * H) as usize, "{dec_name} y plane");
    assert!(frame.u.len() >= (W * H / 4) as usize, "{dec_name} u plane");
    assert!(frame.v.len() >= (W * H / 4) as usize, "{dec_name} v plane");
}

#[test]
#[cfg(feature = "ffmpeg")]
fn ffmpeg_decodes_annexb_clip() -> anyhow::Result<()> {
    let dec = open(Some("ffmpeg"))?;
    assert_eq!(dec.name(), "ffmpeg");
    let frame = first_frame(dec)?.expect("ffmpeg decoded a frame");
    assert_frame("ffmpeg", frame);
    Ok(())
}

#[test]
#[cfg(feature = "openh264")]
fn openh264_decodes_annexb_clip() -> anyhow::Result<()> {
    let dec = open(Some("openh264"))?;
    assert_eq!(dec.name(), "openh264");
    let frame = first_frame(dec)?.expect("openh264 decoded a frame");
    assert_frame("openh264", frame);
    Ok(())
}

#[test]
fn open_prefers_compiled_backend() -> anyhow::Result<()> {
    let dec = open(None)?;
    let name = dec.name();
    assert!(krapy_decode::available().contains(&name));
    let frame = first_frame(dec)?.expect("default backend decoded a frame");
    assert_frame(name, frame);
    Ok(())
}
use krapy_decode::Frame;
use qrcode::QrCode;
use qrcode::types::Color;

const QUIET: usize = 4;
const SCALE: usize = 8;

pub fn pair_qr_frame(uri: &str) -> anyhow::Result<Frame> {
    let code = QrCode::new(uri.as_bytes())?;
    let modules = code.width();
    let mut px = (modules + 2 * QUIET) * SCALE;
    if !px.is_multiple_of(2) {
        px += 1;
    }
    let width = px as u32;
    let height = px as u32;
    let stride_y = px;
    let stride_uv = px / 2;
    let y_len = stride_y * px;
    let uv_len = stride_uv * (px / 2);
    let mut y = vec![255u8; y_len];
    for row in 0..px {
        for col in 0..px {
            let mx = col / SCALE;
            let my = row / SCALE;
            let lum = if mx < QUIET || my < QUIET || mx >= QUIET + modules || my >= QUIET + modules
            {
                255u8
            } else {
                let qx = mx - QUIET;
                let qy = my - QUIET;
                if code[(qx, qy)] == Color::Dark {
                    0
                } else {
                    255
                }
            };
            y[row * stride_y + col] = lum;
        }
    }
    let u = vec![128u8; uv_len];
    let v = vec![128u8; uv_len];
    Ok(Frame {
        width,
        height,
        y,
        u,
        v,
        stride_y,
        stride_uv,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn even_dimensions() {
        let f = pair_qr_frame("krapy://pair?v=1&h=127.0.0.1&p=27183&k=abc").expect("qr");
        assert_eq!(f.width % 2, 0);
        assert_eq!(f.height % 2, 0);
        assert_eq!(f.y.len(), f.stride_y * f.height as usize);
        assert_eq!(f.u.len(), f.stride_uv * (f.height as usize / 2));
    }

    #[test]
    fn has_dark_modules() {
        let f = pair_qr_frame("krapy://pair?v=1&h=127.0.0.1&p=27183&k=abc").expect("qr");
        assert!(f.y.contains(&0));
        assert!(f.y.contains(&255));
    }
}

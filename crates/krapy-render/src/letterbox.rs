#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Viewport {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

pub fn effective_video(vw: u32, vh: u32, rot: u8) -> (f32, f32) {
    if rot % 2 == 1 {
        (vh as f32, vw as f32)
    } else {
        (vw as f32, vh as f32)
    }
}

pub fn letterbox_viewport(win_w: f32, win_h: f32, vid_w: f32, vid_h: f32) -> Viewport {
    if win_w <= 0.0 || win_h <= 0.0 || vid_w <= 0.0 || vid_h <= 0.0 {
        return Viewport {
            x: 0.0,
            y: 0.0,
            w: win_w.max(0.0),
            h: win_h.max(0.0),
        };
    }
    let scale = (win_w / vid_w).min(win_h / vid_h);
    let w = vid_w * scale;
    let h = vid_h * scale;
    Viewport {
        x: (win_w - w) * 0.5,
        y: (win_h - h) * 0.5,
        w,
        h,
    }
}

pub fn display_scale(win_w: f32, win_h: f32, vid_w: f32, vid_h: f32) -> f32 {
    if vid_w <= 0.0 || vid_h <= 0.0 {
        return 1.0;
    }
    (win_w / vid_w).min(win_h / vid_h)
}

pub fn window_to_video(
    px: f32,
    py: f32,
    win_w: f32,
    win_h: f32,
    vid_w: u32,
    vid_h: u32,
    rot: u8,
) -> Option<(u16, u16)> {
    let (evw, evh) = effective_video(vid_w, vid_h, rot);
    let vp = letterbox_viewport(win_w, win_h, evw, evh);
    if px < vp.x || py < vp.y || px >= vp.x + vp.w || py >= vp.y + vp.h {
        return None;
    }
    let fx = (px - vp.x) / vp.w;
    let fy = (py - vp.y) / vp.h;
    let (tu, tv) = match rot % 4 {
        0 => (fx, fy),
        1 => (fy, 1.0 - fx),
        2 => (1.0 - fx, 1.0 - fy),
        3 => (1.0 - fy, fx),
        _ => (fx, fy),
    };
    let x = (tu * vid_w as f32).clamp(0.0, vid_w as f32 - 1.0) as u16;
    let y = (tv * vid_h as f32).clamp(0.0, vid_h as f32 - 1.0) as u16;
    Some((x, y))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn letterbox_wider_window() {
        let vp = letterbox_viewport(800.0, 600.0, 400.0, 800.0);
        assert!((vp.w - 300.0).abs() < 0.01);
        assert!((vp.h - 600.0).abs() < 0.01);
        assert!((vp.x - 250.0).abs() < 0.01);
        assert!((vp.y - 0.0).abs() < 0.01);
    }

    #[test]
    fn letterbox_taller_window() {
        let vp = letterbox_viewport(400.0, 800.0, 400.0, 800.0);
        assert!((vp.w - 400.0).abs() < 0.01);
        assert!((vp.h - 800.0).abs() < 0.01);
    }

    #[test]
    fn map_center_and_edges() {
        let (x, y) = window_to_video(400.0, 400.0, 800.0, 800.0, 400, 800, 0).expect("c");
        assert_eq!((x, y), (200, 400));
        assert!(window_to_video(0.0, 0.0, 800.0, 800.0, 400, 800, 0).is_none());
        let (x, y) = window_to_video(599.0, 799.0, 800.0, 800.0, 400, 800, 0).expect("br");
        assert_eq!((x, y), (399, 799));
    }

    #[test]
    fn rot_swaps_letterbox() {
        let vp = letterbox_viewport(400.0, 800.0, 800.0, 400.0);
        assert!((vp.w - 400.0).abs() < 0.01);
        assert!((vp.h - 200.0).abs() < 0.01);
    }

    #[test]
    fn pointer_all_rotations() {
        let (ww, wh) = (100.0f32, 100.0f32);
        let (vw, vh) = (20u32, 40u32);
        assert_eq!(
            window_to_video(50.0, 50.0, ww, wh, vw, vh, 0).expect("c"),
            (10, 20)
        );
        assert_eq!(
            window_to_video(50.0, 50.0, ww, wh, vw, vh, 1).expect("c"),
            (10, 20)
        );
        assert_eq!(
            window_to_video(50.0, 50.0, ww, wh, vw, vh, 2).expect("c"),
            (10, 20)
        );
        assert_eq!(
            window_to_video(50.0, 50.0, ww, wh, vw, vh, 3).expect("c"),
            (10, 20)
        );
        assert_eq!(
            window_to_video(25.0, 0.0, ww, wh, vw, vh, 0).expect("tl"),
            (0, 0)
        );
        assert_eq!(
            window_to_video(0.0, 25.0, ww, wh, vw, vh, 1).expect("r1"),
            (0, 39)
        );
        assert_eq!(
            window_to_video(74.0, 99.0, ww, wh, vw, vh, 2).expect("r2"),
            (0, 0)
        );
        assert_eq!(
            window_to_video(0.0, 25.0, ww, wh, vw, vh, 3).expect("r3"),
            (19, 0)
        );
    }
}

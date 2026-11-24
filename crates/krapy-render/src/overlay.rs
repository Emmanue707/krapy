use crate::font::{GLYPH_H, GLYPH_W, glyph_rows};
use krapy_transport::Control;
use krapy_transport::wire::SET_KEY_CLIPBOARD;
use krapy_transport::wire::SET_KEY_DISCONNECT;
use krapy_transport::wire::SET_KEY_KEEP_AWAKE;
use krapy_transport::wire::SET_KEY_PAUSE;
use krapy_transport::wire::SET_KEY_PRESET;
use krapy_transport::wire::SET_KEY_ROTATE;
use krapy_transport::wire::SET_KEY_SOUND_PC;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PhoneSettings {
    pub paused: bool,
    pub preset: u8,
    pub sound_pc: bool,
    pub keep_awake: bool,
    pub clipboard: bool,
    pub rotation: u8,
}

impl PhoneSettings {
    pub fn from_control(c: &Control) -> Option<Self> {
        let Control::Settings {
            paused,
            preset,
            sound_pc,
            keep_awake,
            clipboard,
            rotation,
        } = c
        else {
            return None;
        };
        Some(Self {
            paused: *paused != 0,
            preset: *preset,
            sound_pc: *sound_pc != 0,
            keep_awake: *keep_awake != 0,
            clipboard: *clipboard != 0,
            rotation: *rotation & 3,
        })
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct MenuStats {
    pub fps: u8,
    pub latency_ms: u16,
    pub decode_ms: u16,
    pub bitrate_bps: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuRow {
    Pause,
    Preset,
    Sound,
    KeepAwake,
    Clipboard,
    Rotate,
    Disconnect,
}

const ROWS: usize = 7;
const PAD: f32 = 8.0;
const ROW_H: f32 = 14.0;

pub fn ui_scale(window_h: f32) -> u32 {
    let s = (window_h / 480.0).floor() as u32;
    s.clamp(1, 4)
}

pub struct PanelLayout {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub scale: u32,
}

pub fn panel_layout(_win_w: f32, win_h: f32) -> PanelLayout {
    let scale = ui_scale(win_h);
    let cw = 220.0 * scale as f32;
    let ch = (PAD * 2.0 + ROW_H * (ROWS as f32 + 1.0)) * scale as f32;
    PanelLayout {
        x: PAD,
        y: PAD,
        w: cw,
        h: ch,
        scale,
    }
}

pub fn hit_test(px: f32, py: f32, layout: &PanelLayout) -> Option<MenuRow> {
    if px < layout.x || py < layout.y || px >= layout.x + layout.w || py >= layout.y + layout.h {
        return None;
    }
    let row = ((py - layout.y - PAD * layout.scale as f32) / (ROW_H * layout.scale as f32)) as i32;
    if row < 0 || row >= ROWS as i32 {
        return None;
    }
    Some(match row {
        0 => MenuRow::Pause,
        1 => MenuRow::Preset,
        2 => MenuRow::Sound,
        3 => MenuRow::KeepAwake,
        4 => MenuRow::Clipboard,
        5 => MenuRow::Rotate,
        _ => MenuRow::Disconnect,
    })
}

pub fn point_in_panel(px: f32, py: f32, layout: &PanelLayout) -> bool {
    px >= layout.x && py >= layout.y && px < layout.x + layout.w && py < layout.y + layout.h
}

#[derive(Clone, Copy)]
pub struct OverlayQuad {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

pub fn preset_label(id: u8) -> &'static str {
    match id {
        0 => "Smooth",
        2 => "Sharp",
        _ => "Balanced",
    }
}

#[allow(clippy::too_many_arguments)]
pub fn push_text(
    out: &mut Vec<OverlayQuad>,
    x: f32,
    y: f32,
    scale: u32,
    text: &str,
    r: f32,
    g: f32,
    b: f32,
    a: f32,
) {
    let mut cx = x;
    let ch = GLYPH_H as f32 * scale as f32;
    let cw = GLYPH_W as f32 * scale as f32;
    for chc in text.chars() {
        if chc == '\n' {
            cx = x;
            continue;
        }
        let rows = glyph_rows(chc);
        for (yi, row) in rows.iter().enumerate() {
            for xi in 0..GLYPH_W {
                if row & (1 << (GLYPH_W - 1 - xi)) != 0 {
                    out.push(OverlayQuad {
                        x: cx + xi as f32 * scale as f32,
                        y: y + yi as f32 * scale as f32,
                        w: scale as f32,
                        h: scale as f32,
                        r,
                        g,
                        b,
                        a,
                    });
                }
            }
        }
        cx += cw;
    }
    let _ = ch;
}

fn push_rect(
    out: &mut Vec<OverlayQuad>,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    rgba: (f32, f32, f32, f32),
) {
    out.push(OverlayQuad {
        x,
        y,
        w,
        h,
        r: rgba.0,
        g: rgba.1,
        b: rgba.2,
        a: rgba.3,
    });
}

pub fn build_menu_quads(
    out: &mut Vec<OverlayQuad>,
    settings: &PhoneSettings,
    stats: &MenuStats,
    win_w: f32,
    win_h: f32,
) {
    out.clear();
    let layout = panel_layout(win_w, win_h);
    let sc = layout.scale as f32;
    push_rect(
        out,
        layout.x,
        layout.y,
        layout.w,
        layout.h,
        (0.08, 0.08, 0.1, 0.82),
    );
    let mut y = layout.y + PAD * sc;
    let tx = layout.x + PAD * sc;
    let pause_l = if settings.paused {
        "Pause  Alt+P"
    } else {
        "Resume  Alt+P"
    };
    push_text(out, tx, y, layout.scale, pause_l, 1.0, 1.0, 1.0, 1.0);
    y += ROW_H * sc;
    let qual = format!("Quality: {}  Alt+1/2/3", preset_label(settings.preset));
    push_text(out, tx, y, layout.scale, &qual, 1.0, 1.0, 1.0, 1.0);
    y += ROW_H * sc;
    let snd = if settings.sound_pc {
        "Sound: PC  Alt+S"
    } else {
        "Sound: phone  Alt+S"
    };
    push_text(out, tx, y, layout.scale, snd, 1.0, 1.0, 1.0, 1.0);
    y += ROW_H * sc;
    let wake = if settings.keep_awake {
        "Keep awake: on  Alt+K"
    } else {
        "Keep awake: off  Alt+K"
    };
    push_text(out, tx, y, layout.scale, wake, 1.0, 1.0, 1.0, 1.0);
    y += ROW_H * sc;
    let clip = if settings.clipboard {
        "Clipboard: on  Alt+B"
    } else {
        "Clipboard: off  Alt+B"
    };
    push_text(out, tx, y, layout.scale, clip, 1.0, 1.0, 1.0, 1.0);
    y += ROW_H * sc;
    push_text(
        out,
        tx,
        y,
        layout.scale,
        "Rotate  Alt+R",
        1.0,
        1.0,
        1.0,
        1.0,
    );
    y += ROW_H * sc;
    push_text(
        out,
        tx,
        y,
        layout.scale,
        "Disconnect  Alt+D",
        1.0,
        1.0,
        1.0,
        1.0,
    );
    y += ROW_H * sc;
    let mbps = stats.bitrate_bps as f64 / 1_000_000.0;
    let stat = format!(
        "{} fps  {} ms  {:.1} Mbps",
        stats.fps, stats.latency_ms, mbps
    );
    push_text(out, tx, y, layout.scale, &stat, 0.75, 0.85, 1.0, 1.0);
}

pub fn build_paused_tag(out: &mut Vec<OverlayQuad>, win_w: f32, win_h: f32) {
    out.clear();
    let scale = ui_scale(win_h);
    let tx = win_w - (64.0 * scale as f32) - PAD;
    let ty = win_h - (12.0 * scale as f32) - PAD;
    push_rect(
        out,
        tx - PAD,
        ty - PAD / 2.0,
        56.0 * scale as f32,
        14.0 * scale as f32,
        (0.1, 0.1, 0.12, 0.75),
    );
    push_text(out, tx, ty, scale, "paused", 1.0, 0.85, 0.4, 1.0);
}

pub fn row_action(row: MenuRow, settings: &PhoneSettings) -> Option<Control> {
    Some(match row {
        MenuRow::Pause => Control::SetSetting {
            key: SET_KEY_PAUSE,
            value: u8::from(!settings.paused),
        },
        MenuRow::Preset => Control::SetSetting {
            key: SET_KEY_PRESET,
            value: (settings.preset + 1) % 3,
        },
        MenuRow::Sound => Control::SetSetting {
            key: SET_KEY_SOUND_PC,
            value: u8::from(!settings.sound_pc),
        },
        MenuRow::KeepAwake => Control::SetSetting {
            key: SET_KEY_KEEP_AWAKE,
            value: u8::from(!settings.keep_awake),
        },
        MenuRow::Clipboard => Control::SetSetting {
            key: SET_KEY_CLIPBOARD,
            value: u8::from(!settings.clipboard),
        },
        MenuRow::Rotate => Control::SetSetting {
            key: SET_KEY_ROTATE,
            value: 0,
        },
        MenuRow::Disconnect => Control::SetSetting {
            key: SET_KEY_DISCONNECT,
            value: 0,
        },
    })
}

#[allow(dead_code)]
pub fn preset_shortcut(digit: u8) -> Option<Control> {
    let preset = match digit {
        1 => 0,
        2 => 1,
        3 => 2,
        _ => return None,
    };
    Some(Control::SetSetting {
        key: SET_KEY_PRESET,
        value: preset,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hit_test_rows() {
        let layout = panel_layout(800.0, 600.0);
        let row0 = hit_test(layout.x + 4.0, layout.y + 4.0, &layout);
        assert_eq!(row0, Some(MenuRow::Pause));
        let row2 = hit_test(
            layout.x + 4.0,
            layout.y + PAD * layout.scale as f32 + ROW_H * layout.scale as f32 * 2.5,
            &layout,
        );
        assert_eq!(row2, Some(MenuRow::Sound));
        assert!(hit_test(layout.x + layout.w + 10.0, layout.y, &layout).is_none());
    }
}

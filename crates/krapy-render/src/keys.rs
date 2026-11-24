use krapy_transport::Control;
use krapy_transport::wire::{
    SET_KEY_CLIPBOARD, SET_KEY_DISCONNECT, SET_KEY_KEEP_AWAKE, SET_KEY_PAUSE, SET_KEY_PRESET,
    SET_KEY_ROTATE, SET_KEY_SOUND_PC,
};
use winit::event::KeyEvent;
use winit::keyboard::ModifiersState;
use winit::keyboard::{Key, NamedKey};

pub const KEY_BACK: u8 = 1;
pub const KEY_HOME: u8 = 2;
#[allow(dead_code)]
pub const KEY_RECENTS: u8 = 3;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UiAction {
    ToggleMenu,
    Setting(Control),
    LocalRotate,
}

pub fn named_key_action(key: &Key) -> Option<u8> {
    match key {
        Key::Named(NamedKey::Escape) => Some(KEY_BACK),
        Key::Named(NamedKey::Home) => Some(KEY_HOME),
        _ => None,
    }
}

pub fn pc_shortcut(event: &KeyEvent, mods: ModifiersState) -> Option<UiAction> {
    if mods.control_key() {
        if matches!(event.logical_key, Key::Character(ref s) if s.eq_ignore_ascii_case("r")) {
            return Some(UiAction::LocalRotate);
        }
        return None;
    }
    if mods.alt_key() && !mods.control_key() && !mods.super_key() {
        return alt_combo(&event.logical_key);
    }
    if matches!(event.logical_key, Key::Named(NamedKey::F1)) {
        return Some(UiAction::ToggleMenu);
    }
    None
}

pub(crate) fn alt_combo(key: &Key) -> Option<UiAction> {
    let ch = match key {
        Key::Character(s) => s.chars().next()?,
        _ => return None,
    };
    let c = ch.to_ascii_lowercase();
    Some(match c {
        'm' => UiAction::ToggleMenu,
        'p' => UiAction::Setting(Control::SetSetting {
            key: SET_KEY_PAUSE,
            value: 0xff,
        }),
        '1' => UiAction::Setting(Control::SetSetting {
            key: SET_KEY_PRESET,
            value: 0,
        }),
        '2' => UiAction::Setting(Control::SetSetting {
            key: SET_KEY_PRESET,
            value: 1,
        }),
        '3' => UiAction::Setting(Control::SetSetting {
            key: SET_KEY_PRESET,
            value: 2,
        }),
        's' => UiAction::Setting(Control::SetSetting {
            key: SET_KEY_SOUND_PC,
            value: 0xff,
        }),
        'k' => UiAction::Setting(Control::SetSetting {
            key: SET_KEY_KEEP_AWAKE,
            value: 0xff,
        }),
        'b' => UiAction::Setting(Control::SetSetting {
            key: SET_KEY_CLIPBOARD,
            value: 0xff,
        }),
        'r' => UiAction::Setting(Control::SetSetting {
            key: SET_KEY_ROTATE,
            value: 0,
        }),
        'd' => UiAction::Setting(Control::SetSetting {
            key: SET_KEY_DISCONNECT,
            value: 0,
        }),
        _ => return None,
    })
}

pub fn resolve_toggle_setting(ctrl: Control, settings: &crate::overlay::PhoneSettings) -> Control {
    match ctrl {
        Control::SetSetting { key, value: 0xff } => {
            let v = match key {
                SET_KEY_PAUSE => u8::from(!settings.paused),
                SET_KEY_SOUND_PC => u8::from(!settings.sound_pc),
                SET_KEY_KEEP_AWAKE => u8::from(!settings.keep_awake),
                SET_KEY_CLIPBOARD => u8::from(!settings.clipboard),
                _ => 0,
            };
            Control::SetSetting { key, value: v }
        }
        other => other,
    }
}

pub fn consumes_pc_shortcut(event: &KeyEvent, mods: ModifiersState) -> bool {
    pc_shortcut(event, mods).is_some()
        || (mods.alt_key() && matches!(event.logical_key, Key::Character(_)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use winit::keyboard::NamedKey;

    #[test]
    fn key_table() {
        assert_eq!(
            named_key_action(&Key::Named(NamedKey::Escape)),
            Some(KEY_BACK)
        );
        assert_eq!(
            named_key_action(&Key::Named(NamedKey::Home)),
            Some(KEY_HOME)
        );
        assert_eq!(named_key_action(&Key::Named(NamedKey::Space)), None);
    }

    #[test]
    fn alt_combos_consumed() {
        assert!(alt_combo(&Key::Character("p".into())).is_some());
        assert!(alt_combo(&Key::Character("d".into())).is_some());
        assert!(alt_combo(&Key::Character("q".into())).is_none());
    }
}

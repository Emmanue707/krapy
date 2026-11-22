use winit::keyboard::{Key, NamedKey};

pub const KEY_BACK: u8 = 1;
pub const KEY_HOME: u8 = 2;
pub const KEY_RECENTS: u8 = 3;

pub fn named_key_action(key: &Key) -> Option<u8> {
    match key {
        Key::Named(NamedKey::Escape) => Some(KEY_BACK),
        Key::Named(NamedKey::Home) => Some(KEY_HOME),
        Key::Named(NamedKey::F1) => Some(KEY_RECENTS),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use winit::keyboard::NamedKey;

    #[test]
    fn key_table() {
        assert_eq!(named_key_action(&Key::Named(NamedKey::Escape)), Some(KEY_BACK));
        assert_eq!(named_key_action(&Key::Named(NamedKey::Home)), Some(KEY_HOME));
        assert_eq!(named_key_action(&Key::Named(NamedKey::F1)), Some(KEY_RECENTS));
        assert_eq!(named_key_action(&Key::Named(NamedKey::Space)), None);
    }
}
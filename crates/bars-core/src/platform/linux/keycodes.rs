//! Canonical key names <-> Linux evdev key codes.

use evdev::KeyCode as K;

use crate::hotkey::Mods;

pub const MODIFIERS: &[(K, Mods)] = &[
    (K::KEY_LEFTCTRL, Mods::CTRL),
    (K::KEY_RIGHTCTRL, Mods::CTRL),
    (K::KEY_LEFTALT, Mods::ALT),
    (K::KEY_RIGHTALT, Mods::ALT),
    (K::KEY_LEFTSHIFT, Mods::SHIFT),
    (K::KEY_RIGHTSHIFT, Mods::SHIFT),
    (K::KEY_LEFTMETA, Mods::SUPER),
    (K::KEY_RIGHTMETA, Mods::SUPER),
];

pub fn modifier_of(code: K) -> Option<Mods> {
    MODIFIERS.iter().find(|(k, _)| *k == code).map(|(_, m)| *m)
}

const NAMED: &[(&str, K)] = &[
    ("Space", K::KEY_SPACE),
    ("Tab", K::KEY_TAB),
    ("Enter", K::KEY_ENTER),
    ("Backspace", K::KEY_BACKSPACE),
    ("Escape", K::KEY_ESC),
    ("Insert", K::KEY_INSERT),
    ("Delete", K::KEY_DELETE),
    ("Home", K::KEY_HOME),
    ("End", K::KEY_END),
    ("Pageup", K::KEY_PAGEUP),
    ("Pagedown", K::KEY_PAGEDOWN),
    ("Up", K::KEY_UP),
    ("Down", K::KEY_DOWN),
    ("Left", K::KEY_LEFT),
    ("Right", K::KEY_RIGHT),
    ("Grave", K::KEY_GRAVE),
    ("Minus", K::KEY_MINUS),
    ("Equal", K::KEY_EQUAL),
    ("Leftbracket", K::KEY_LEFTBRACE),
    ("Rightbracket", K::KEY_RIGHTBRACE),
    ("Backslash", K::KEY_BACKSLASH),
    ("Semicolon", K::KEY_SEMICOLON),
    ("Apostrophe", K::KEY_APOSTROPHE),
    ("Comma", K::KEY_COMMA),
    ("Period", K::KEY_DOT),
    ("Slash", K::KEY_SLASH),
    ("Capslock", K::KEY_CAPSLOCK),
    ("Menu", K::KEY_COMPOSE),
    ("Pause", K::KEY_PAUSE),
    ("Scrolllock", K::KEY_SCROLLLOCK),
    ("Print", K::KEY_SYSRQ),
];

const LETTERS: [K; 26] = [
    K::KEY_A, K::KEY_B, K::KEY_C, K::KEY_D, K::KEY_E, K::KEY_F, K::KEY_G, K::KEY_H, K::KEY_I,
    K::KEY_J, K::KEY_K, K::KEY_L, K::KEY_M, K::KEY_N, K::KEY_O, K::KEY_P, K::KEY_Q, K::KEY_R,
    K::KEY_S, K::KEY_T, K::KEY_U, K::KEY_V, K::KEY_W, K::KEY_X, K::KEY_Y, K::KEY_Z,
];
const DIGITS: [K; 10] = [
    K::KEY_0, K::KEY_1, K::KEY_2, K::KEY_3, K::KEY_4, K::KEY_5, K::KEY_6, K::KEY_7, K::KEY_8,
    K::KEY_9,
];
const FKEYS: [K; 24] = [
    K::KEY_F1, K::KEY_F2, K::KEY_F3, K::KEY_F4, K::KEY_F5, K::KEY_F6, K::KEY_F7, K::KEY_F8,
    K::KEY_F9, K::KEY_F10, K::KEY_F11, K::KEY_F12, K::KEY_F13, K::KEY_F14, K::KEY_F15,
    K::KEY_F16, K::KEY_F17, K::KEY_F18, K::KEY_F19, K::KEY_F20, K::KEY_F21, K::KEY_F22,
    K::KEY_F23, K::KEY_F24,
];

pub fn key_from_name(name: &str) -> Option<K> {
    let name = crate::hotkey::canonical_key_name(name);
    if let Some((_, k)) = NAMED.iter().find(|(n, _)| *n == name) {
        return Some(*k);
    }
    let mut chars = name.chars();
    if let (Some(c), None) = (chars.next(), chars.next()) {
        if c.is_ascii_uppercase() {
            return Some(LETTERS[(c as u8 - b'A') as usize]);
        }
        if c.is_ascii_digit() {
            return Some(DIGITS[(c as u8 - b'0') as usize]);
        }
    }
    if let Some(n) = name.strip_prefix('F').and_then(|n| n.parse::<usize>().ok()) {
        return FKEYS.get(n.checked_sub(1)?).copied();
    }
    None
}

pub fn name_from_key(code: K) -> Option<String> {
    if let Some((n, _)) = NAMED.iter().find(|(_, k)| *k == code) {
        return Some((*n).to_string());
    }
    if let Some(i) = LETTERS.iter().position(|k| *k == code) {
        return Some(((b'A' + i as u8) as char).to_string());
    }
    if let Some(i) = DIGITS.iter().position(|k| *k == code) {
        return Some(((b'0' + i as u8) as char).to_string());
    }
    if let Some(i) = FKEYS.iter().position(|k| *k == code) {
        return Some(format!("F{}", i + 1));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_roundtrip() {
        for name in ["Space", "A", "z", "7", "F9", "f24", "Grave", "Pagedown"] {
            let k = key_from_name(name).unwrap_or_else(|| panic!("{name}"));
            assert_eq!(
                name_from_key(k).unwrap(),
                crate::hotkey::canonical_key_name(name)
            );
        }
        assert!(key_from_name("F25").is_none());
        assert!(key_from_name("Nope").is_none());
    }
}

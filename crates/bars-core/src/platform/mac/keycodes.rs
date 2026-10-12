//! Canonical key names <-> macOS virtual key codes (kVK_*).

use crate::hotkey::{KeyMap, Mods};

pub const KVK_ESCAPE: u32 = 0x35;

const KEYS: &[(&str, u32)] = &[
    ("A", 0x00), ("S", 0x01), ("D", 0x02), ("F", 0x03), ("H", 0x04), ("G", 0x05), ("Z", 0x06),
    ("X", 0x07), ("C", 0x08), ("V", 0x09), ("B", 0x0B), ("Q", 0x0C), ("W", 0x0D), ("E", 0x0E),
    ("R", 0x0F), ("Y", 0x10), ("T", 0x11), ("1", 0x12), ("2", 0x13), ("3", 0x14), ("4", 0x15),
    ("6", 0x16), ("5", 0x17), ("Equal", 0x18), ("9", 0x19), ("7", 0x1A), ("Minus", 0x1B),
    ("8", 0x1C), ("0", 0x1D), ("Rightbracket", 0x1E), ("O", 0x1F), ("U", 0x20),
    ("Leftbracket", 0x21), ("I", 0x22), ("P", 0x23), ("Enter", 0x24), ("L", 0x25), ("J", 0x26),
    ("Apostrophe", 0x27), ("K", 0x28), ("Semicolon", 0x29), ("Backslash", 0x2A), ("Comma", 0x2B),
    ("Slash", 0x2C), ("N", 0x2D), ("M", 0x2E), ("Period", 0x2F), ("Tab", 0x30), ("Space", 0x31),
    ("Grave", 0x32), ("Backspace", 0x33), ("Escape", 0x35), ("Capslock", 0x39),
    ("F17", 0x40), ("F18", 0x4F), ("F19", 0x50), ("F20", 0x5A), ("F5", 0x60), ("F6", 0x61),
    ("F7", 0x62), ("F3", 0x63), ("F8", 0x64), ("F9", 0x65), ("F11", 0x67), ("F13", 0x69),
    ("F16", 0x6A), ("F14", 0x6B), ("F10", 0x6D), ("F12", 0x6F), ("F15", 0x71), ("Insert", 0x72),
    ("Home", 0x73), ("Pageup", 0x74), ("Delete", 0x75), ("F4", 0x76), ("End", 0x77), ("F2", 0x78),
    ("Pagedown", 0x79), ("F1", 0x7A), ("Left", 0x7B), ("Right", 0x7C), ("Down", 0x7D), ("Up", 0x7E),
];

/// Modifier key codes: Command, Shift, Option, Control (left/right).
pub const MODIFIERS: &[(u32, Mods)] = &[
    (0x37, Mods::SUPER),
    (0x36, Mods::SUPER),
    (0x38, Mods::SHIFT),
    (0x3C, Mods::SHIFT),
    (0x3A, Mods::ALT),
    (0x3D, Mods::ALT),
    (0x3B, Mods::CTRL),
    (0x3E, Mods::CTRL),
    // kVK_Function: Fn / Globe.
    (0x3F, Mods::FN),
];

pub struct MacKeys;

impl KeyMap for MacKeys {
    fn modifier_of(&self, code: u32) -> Option<Mods> {
        MODIFIERS.iter().find(|(c, _)| *c == code).map(|(_, m)| *m)
    }

    fn name_of(&self, code: u32) -> Option<String> {
        KEYS.iter().find(|(_, c)| *c == code).map(|(n, _)| (*n).to_string())
    }

    fn code_of(&self, name: &str) -> Option<u32> {
        let name = crate::hotkey::canonical_key_name(name);
        KEYS.iter().find(|(n, _)| *n == name).map(|(_, c)| *c)
    }

    fn is_escape(&self, code: u32) -> bool {
        code == KVK_ESCAPE
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_roundtrip() {
        for name in ["Space", "A", "7", "F9", "Grave", "Pagedown"] {
            let c = MacKeys.code_of(name).unwrap_or_else(|| panic!("{name}"));
            assert_eq!(MacKeys.name_of(c).unwrap(), name);
        }
    }
}

//! Canonical key names <-> Windows virtual-key codes.

use crate::hotkey::{KeyMap, Mods};

pub const VK_ESCAPE: u32 = 0x1B;
/// An unassigned virtual key, tapped to keep Windows from opening the Start
/// menu when a chord containing the Windows key is released.
pub const VK_UNASSIGNED: u16 = 0xE8;

const NAMED: &[(&str, u32)] = &[
    ("Space", 0x20),
    ("Tab", 0x09),
    ("Enter", 0x0D),
    ("Backspace", 0x08),
    ("Escape", 0x1B),
    ("Insert", 0x2D),
    ("Delete", 0x2E),
    ("Home", 0x24),
    ("End", 0x23),
    ("Pageup", 0x21),
    ("Pagedown", 0x22),
    ("Left", 0x25),
    ("Up", 0x26),
    ("Right", 0x27),
    ("Down", 0x28),
    ("Grave", 0xC0),
    ("Minus", 0xBD),
    ("Equal", 0xBB),
    ("Leftbracket", 0xDB),
    ("Rightbracket", 0xDD),
    ("Backslash", 0xDC),
    ("Semicolon", 0xBA),
    ("Apostrophe", 0xDE),
    ("Comma", 0xBC),
    ("Period", 0xBE),
    ("Slash", 0xBF),
    ("Capslock", 0x14),
    ("Menu", 0x5D),
    ("Pause", 0x13),
    ("Scrolllock", 0x91),
    ("Print", 0x2C),
];

pub struct WinKeys;

impl KeyMap for WinKeys {
    fn modifier_of(&self, vk: u32) -> Option<Mods> {
        match vk {
            0xA0 | 0xA1 | 0x10 => Some(Mods::SHIFT),
            0xA2 | 0xA3 | 0x11 => Some(Mods::CTRL),
            0xA4 | 0xA5 | 0x12 => Some(Mods::ALT),
            0x5B | 0x5C => Some(Mods::SUPER),
            _ => None,
        }
    }

    fn name_of(&self, vk: u32) -> Option<String> {
        if let Some((n, _)) = NAMED.iter().find(|(_, c)| *c == vk) {
            return Some((*n).to_string());
        }
        match vk {
            0x41..=0x5A | 0x30..=0x39 => Some((vk as u8 as char).to_string()),
            0x70..=0x87 => Some(format!("F{}", vk - 0x70 + 1)),
            _ => None,
        }
    }

    fn code_of(&self, name: &str) -> Option<u32> {
        let name = crate::hotkey::canonical_key_name(name);
        if let Some((_, c)) = NAMED.iter().find(|(n, _)| *n == name) {
            return Some(*c);
        }
        let mut chars = name.chars();
        if let (Some(c), None) = (chars.next(), chars.next()) {
            if c.is_ascii_uppercase() || c.is_ascii_digit() {
                return Some(c as u32);
            }
        }
        let n: u32 = name.strip_prefix('F')?.parse().ok()?;
        (1..=24).contains(&n).then(|| 0x70 + n - 1)
    }

    fn is_escape(&self, vk: u32) -> bool {
        vk == VK_ESCAPE
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_roundtrip() {
        for name in ["Space", "A", "7", "F9", "F24", "Grave", "Pagedown"] {
            let c = WinKeys.code_of(name).unwrap_or_else(|| panic!("{name}"));
            assert_eq!(WinKeys.name_of(c).unwrap(), name);
        }
    }
}

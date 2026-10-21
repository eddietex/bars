//! Canonical key names <-> Windows virtual-key codes, mouse buttons
//! included (Windows has virtual keys for them).

use crate::hotkey::{Key, KeyMap, Mods};

pub const VK_ESCAPE: u32 = 0x1B;
/// An unassigned virtual key, tapped to keep Windows from opening the Start
/// menu (or an app its menu bar) when a chord containing the Windows key
/// (or Alt) is released.
pub const VK_UNASSIGNED: u16 = 0xE8;
pub const VK_MBUTTON: u32 = 0x04;
pub const VK_XBUTTON1: u32 = 0x05;
pub const VK_XBUTTON2: u32 = 0x06;

/// Mouse buttons (left, right, middle, back, forward) and their virtual
/// keys. Windows reports no further buttons.
const MOUSE: &[(u8, u32)] = &[(1, 0x01), (2, 0x02), (3, VK_MBUTTON), (4, VK_XBUTTON1), (5, VK_XBUTTON2)];

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
    // One side's modifier, for a shortcut that is just that key.
    ("Leftshift", 0xA0),
    ("Rightshift", 0xA1),
    ("Leftctrl", 0xA2),
    ("Rightctrl", 0xA3),
    ("Leftalt", 0xA4),
    ("Rightalt", 0xA5),
    ("Leftsuper", 0x5B),
    ("Rightsuper", 0x5C),
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
        if let Some(button) = self.mouse_button(vk) {
            return Some(Key::mouse(button).0);
        }
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
        if let Some(button) = Key(name.clone()).mouse_button() {
            return MOUSE.iter().find(|(b, _)| *b == button).map(|(_, vk)| *vk);
        }
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

    fn mouse_button(&self, vk: u32) -> Option<u8> {
        MOUSE.iter().find(|(_, v)| *v == vk).map(|(b, _)| *b)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_roundtrip() {
        for name in ["Space", "A", "7", "F9", "F24", "Grave", "Pagedown", "Mouse3", "Mouse4", "Mouse5", "Rightctrl", "Leftsuper"] {
            let c = WinKeys.code_of(name).unwrap_or_else(|| panic!("{name}"));
            assert_eq!(WinKeys.name_of(c).unwrap(), name);
        }
    }
}

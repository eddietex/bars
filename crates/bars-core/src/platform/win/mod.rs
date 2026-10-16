//! Windows backends: a low-level keyboard hook for the global shortcut,
//! SendInput (via enigo) for typing/pasting, the clipboard, and UI
//! Automation for reading the focused field.

pub mod clipboard;
pub mod hotkeys;
pub mod keycodes;
pub mod mute;
pub mod permissions;
pub mod uia;

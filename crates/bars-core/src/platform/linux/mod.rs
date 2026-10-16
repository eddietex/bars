pub mod atspi;
pub mod data_control;
pub mod doctor;
pub mod evdev_hotkeys;
pub mod inject;
pub mod keycodes;
pub mod keymap;
pub mod mute;
pub mod permissions;
pub mod portal;

/// Name of our uinput device; the hotkey listener ignores it.
pub const VIRTUAL_KEYBOARD_NAME: &str = "Bars Virtual Keyboard";

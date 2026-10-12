//! macOS backends: a Quartz event tap for the global shortcut, CGEvent key
//! posting for typing/pasting, the pasteboard, and the accessibility API
//! for reading the focused field.

pub mod ax;
pub mod hotkeys;
pub mod keycodes;
pub mod main_thread;
pub mod paste;
pub mod permissions;
pub mod secure_input;

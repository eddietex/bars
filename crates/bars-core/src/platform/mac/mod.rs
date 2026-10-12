//! macOS backends: a listen-only Quartz event tap for the global shortcut and
//! CGEvent key posting (via enigo) for typing/pasting.

pub mod hotkeys;
pub mod keycodes;
pub mod permissions;

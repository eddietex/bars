//! macOS backends: a Quartz event tap for the global shortcut, CGEvent key
//! posting for typing/pasting, the pasteboard, and the accessibility API
//! for reading the focused field.

pub mod ax;
pub mod hotkeys;
pub mod keycodes;
pub mod main_thread;
pub mod mute;
pub mod paste;
pub mod permissions;
pub mod secure_input;

use std::sync::OnceLock;

/// The macOS version as (major, minor), e.g. (27, 0).
pub fn os_version() -> (u32, u32) {
    static VERSION: OnceLock<(u32, u32)> = OnceLock::new();
    *VERSION.get_or_init(|| {
        let mut buf = [0u8; 32];
        let mut len = buf.len();
        let status = unsafe {
            libc::sysctlbyname(c"kern.osproductversion".as_ptr(), buf.as_mut_ptr().cast(), &mut len, std::ptr::null_mut(), 0)
        };
        let text = if status == 0 { std::str::from_utf8(&buf[..len.min(buf.len())]).unwrap_or("") } else { "" };
        parse_version(text)
    })
}

pub fn os_at_least(major: u32, minor: u32) -> bool {
    os_version() >= (major, minor)
}

fn parse_version(text: &str) -> (u32, u32) {
    let mut parts = text.trim_end_matches('\0').trim().split('.').map(|p| p.parse().unwrap_or(0));
    (parts.next().unwrap_or(0), parts.next().unwrap_or(0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_version() {
        assert_eq!(parse_version("27.2\0"), (27, 2));
        assert_eq!(parse_version("26.5.2"), (26, 5));
        assert_eq!(parse_version("27"), (27, 0));
        assert_eq!(parse_version(""), (0, 0));
        assert!(os_version().0 >= 12);
    }
}

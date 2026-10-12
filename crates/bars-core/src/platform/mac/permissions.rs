//! macOS privacy permissions (TCC) Bars needs.

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGPreflightListenEventAccess() -> bool;
    fn CGRequestListenEventAccess() -> bool;
    fn CGPreflightPostEventAccess() -> bool;
    fn CGRequestPostEventAccess() -> bool;
}

/// Input Monitoring: needed to observe the global shortcut.
pub fn can_listen() -> bool {
    unsafe { CGPreflightListenEventAccess() }
}

/// Accessibility: needed to post key events (paste/type).
pub fn can_post() -> bool {
    unsafe { CGPreflightPostEventAccess() }
}

/// Shows the system prompts (each leads to System Settings).
pub fn request() {
    unsafe {
        if !CGPreflightListenEventAccess() {
            CGRequestListenEventAccess();
        }
        if !CGPreflightPostEventAccess() {
            CGRequestPostEventAccess();
        }
    }
}

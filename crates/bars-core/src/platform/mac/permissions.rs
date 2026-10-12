//! macOS privacy permissions (TCC) Bars needs.

use block2::RcBlock;
use objc2::runtime::Bool;
use objc2_av_foundation::{AVAuthorizationStatus, AVCaptureDevice, AVMediaTypeAudio};

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

/// Shows the Input Monitoring prompt (only the first time; it leads to
/// System Settings).
pub fn request_listen() {
    unsafe {
        if !CGPreflightListenEventAccess() {
            CGRequestListenEventAccess();
        }
    }
}

/// Shows the Accessibility prompt (only the first time).
pub fn request_post() {
    unsafe {
        if !CGPreflightPostEventAccess() {
            CGRequestPostEventAccess();
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum MicAccess {
    Granted,
    /// Not asked yet: requesting shows the system prompt.
    NotAsked,
    /// Denied (or restricted by policy): only System Settings can change it.
    Denied,
}

pub fn microphone() -> MicAccess {
    let Some(audio) = (unsafe { AVMediaTypeAudio }) else {
        return MicAccess::Granted;
    };
    match unsafe { AVCaptureDevice::authorizationStatusForMediaType(audio) } {
        AVAuthorizationStatus::Authorized => MicAccess::Granted,
        AVAuthorizationStatus::NotDetermined => MicAccess::NotAsked,
        _ => MicAccess::Denied,
    }
}

/// Shows the microphone prompt if the user hasn't been asked yet. The
/// answer arrives asynchronously; the setup assistant polls for it.
pub fn request_microphone() {
    if microphone() != MicAccess::NotAsked {
        return;
    }
    let Some(audio) = (unsafe { AVMediaTypeAudio }) else { return };
    let done = RcBlock::new(|granted: Bool| log::info!("microphone access granted: {}", granted.as_bool()));
    unsafe { AVCaptureDevice::requestAccessForMediaType_completionHandler(audio, &done) };
}

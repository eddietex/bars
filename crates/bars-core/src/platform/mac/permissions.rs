//! macOS privacy permissions (TCC) Bars needs: the microphone, and
//! Accessibility, which covers both noticing the shortcut (a listen-only
//! event tap) and typing/pasting (posting key events). Input Monitoring
//! would only cover the first, so Bars doesn't ask for it.

use block2::RcBlock;
use core_foundation::base::TCFType;
use core_foundation::boolean::CFBoolean;
use core_foundation::dictionary::{CFDictionary, CFDictionaryRef};
use core_foundation::string::{CFString, CFStringRef};
use objc2::runtime::Bool;
use objc2_av_foundation::{AVAuthorizationStatus, AVCaptureDevice, AVMediaTypeAudio};

use super::main_thread;

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXIsProcessTrusted() -> bool;
    fn AXIsProcessTrustedWithOptions(options: CFDictionaryRef) -> bool;
    static kAXTrustedCheckOptionPrompt: CFStringRef;
}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGPreflightListenEventAccess() -> bool;
}

/// Accessibility: lets Bars see the shortcut and type into other apps.
pub fn accessibility() -> bool {
    unsafe { AXIsProcessTrusted() }
}

/// Input Monitoring, which an earlier version asked for. It's enough for
/// the shortcut (but not for typing), so it still counts for that.
pub fn input_monitoring() -> bool {
    unsafe { CGPreflightListenEventAccess() }
}

/// Adds Bars to the Accessibility list and, the first time, shows the
/// system prompt. macOS shows that prompt only once, so the caller also
/// opens the settings pane.
pub fn request_accessibility() {
    main_thread::run(|| unsafe {
        let key = CFString::wrap_under_get_rule(kAXTrustedCheckOptionPrompt);
        let options = CFDictionary::from_CFType_pairs(&[(key, CFBoolean::true_value())]);
        AXIsProcessTrustedWithOptions(options.as_concrete_TypeRef());
    });
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
    main_thread::run(|| {
        let done = RcBlock::new(|granted: Bool| log::info!("microphone access granted: {}", granted.as_bool()));
        unsafe { AVCaptureDevice::requestAccessForMediaType_completionHandler(audio, &done) };
    });
}

//! macOS privacy permissions (TCC) Bars needs: the microphone, and
//! Accessibility, which covers both noticing the shortcut (a listen-only
//! event tap) and typing/pasting (posting key events). Input Monitoring
//! would only cover the first, so Bars doesn't ask for it.

use std::sync::atomic::{AtomicBool, Ordering};

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
    fn CGPreflightPostEventAccess() -> bool;
}

/// What System Settings calls the Accessibility permission (macOS 27
/// renamed it; the settings link stayed the same).
pub fn accessibility_pane() -> &'static str {
    if super::os_at_least(27, 0) { "Device Control and Data Access" } else { "Accessibility" }
}

/// Accessibility as macOS reports it. Some macOS 27 machines report it
/// off while it's on and works, so decisions go through [`control`].
pub fn accessibility() -> bool {
    unsafe { AXIsProcessTrusted() }
}

/// Posting key events, which pasting and typing need. Accessibility
/// includes it.
pub fn post_events() -> bool {
    unsafe { CGPreflightPostEventAccess() }
}

static FULL_LISTENER: AtomicBool = AtomicBool::new(false);

/// The shortcut listener started with the access that lets it hold keys
/// back from apps, which only Accessibility gives.
pub fn full_listener() -> bool {
    FULL_LISTENER.load(Ordering::Relaxed)
}

pub(super) fn set_full_listener(on: bool) {
    FULL_LISTENER.store(on, Ordering::Relaxed);
}

/// Accessibility as it works: macOS saying so, or Bars doing what only
/// Accessibility allows.
pub fn control() -> bool {
    granted(accessibility(), post_events(), full_listener())
}

fn granted(reported: bool, post_events: bool, full_listener: bool) -> bool {
    reported || post_events || full_listener
}

/// One line for the log and the doctor.
pub fn summary() -> String {
    format!(
        "accessibility reported {}, posting key events {}, input monitoring {}, shortcut listener {}",
        yes(accessibility()),
        yes(post_events()),
        yes(input_monitoring()),
        if full_listener() { "full" } else { "listen-only or not started" }
    )
}

fn yes(on: bool) -> &'static str {
    if on { "yes" } else { "no" }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn any_sign_of_access_counts() {
        assert!(!granted(false, false, false));
        // macOS 27 can report Accessibility off while Bars pastes and holds keys back.
        assert!(granted(false, true, false));
        assert!(granted(false, false, true));
        assert!(granted(true, false, false));
    }
}

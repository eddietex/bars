//! Text that names what each platform asks for, in its own terms.

#[cfg(target_os = "macos")]
fn pane() -> &'static str {
    bars_core::platform::mac::permissions::accessibility_pane()
}

/// The Home banner while setup isn't finished.
pub fn setup_banner() -> String {
    #[cfg(target_os = "macos")]
    return format!("Download a speech model, then allow the microphone and {} — it takes a minute.", pane());
    #[cfg(target_os = "windows")]
    return "Download a speech model and allow the microphone — it takes a minute.".into();
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    return "Download a speech model and allow keyboard access — it takes a minute.".into();
}

/// The setup assistant's Try step, before the earlier steps are done.
pub fn needs_for_dictation() -> String {
    #[cfg(target_os = "macos")]
    return format!("Finish the earlier steps first. Bars needs the model, the microphone and {} to work.", pane());
    #[cfg(target_os = "windows")]
    return "Finish the earlier steps first. Bars needs the model and the microphone to work.".into();
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    return "Finish the earlier steps first. Bars needs the model and keyboard access to work.".into();
}

/// Under the shortcuts, when no shortcut listener is running.
pub fn shortcut_off() -> String {
    #[cfg(target_os = "macos")]
    return format!("Shortcuts need {} — see Setup.", pane());
    #[cfg(target_os = "windows")]
    return "The shortcut listener couldn't start — see Setup.".into();
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    return "Keyboard access is off — see Setup.".into();
}

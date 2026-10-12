//! Platform integration behind small, OS-neutral facades: global hotkeys,
//! text insertion and permission checks. Windows/macOS implementations slot
//! in next to `linux` without touching the controller or UI.

#[cfg(target_os = "linux")]
pub mod linux;
pub mod requirements;

use crossbeam_channel::Sender;

use crate::config::{HotkeyBackendKind, InsertionConfig};
use crate::hotkey::{Chord, HotkeyEvent, Mods};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Permissions {
    /// Global hotkeys can be observed.
    pub hotkeys: bool,
    /// Text can be typed/pasted into other apps.
    pub keyboard: bool,
    /// The clipboard can be driven programmatically.
    pub clipboard: bool,
    pub sandboxed: bool,
    /// Human-readable command that fixes missing access, when applicable.
    pub fix_command: Option<String>,
    pub can_fix_automatically: bool,
    /// Everything the app needs, with status and fixes (drives onboarding).
    pub items: Vec<requirements::Requirement>,
}

/// Human-readable capability report (`bars --doctor`).
pub fn doctor_report() -> String {
    #[cfg(target_os = "linux")]
    {
        linux::doctor::report()
    }
    #[cfg(not(target_os = "linux"))]
    {
        "diagnostics are not implemented on this platform yet".into()
    }
}

pub fn check_permissions(ctx: &requirements::Context) -> Permissions {
    let items = requirements::check(ctx);
    #[cfg(target_os = "linux")]
    {
        let hotkeys = linux::evdev_hotkeys::can_read_keyboards();
        let keyboard = linux::permissions::can_open_uinput();
        Permissions {
            hotkeys,
            keyboard,
            clipboard: linux::inject::clipboard_available(),
            sandboxed: crate::paths::is_flatpak(),
            fix_command: (!hotkeys || !keyboard).then(linux::permissions::manual_command),
            can_fix_automatically: linux::permissions::can_elevate(),
            items,
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        Permissions {
            items,
            ..Permissions::default()
        }
    }
}

/// Requests the OS-level access Bars needs (Linux: installs a udev rule
/// via polkit).
pub fn fix_permissions() -> anyhow::Result<()> {
    #[cfg(target_os = "linux")]
    {
        linux::permissions::install_with_pkexec()
    }
    #[cfg(not(target_os = "linux"))]
    {
        Ok(())
    }
}

/// The active global-hotkey backend.
pub enum Hotkeys {
    #[cfg(target_os = "linux")]
    Evdev(linux::evdev_hotkeys::EvdevHotkeys),
    #[cfg(target_os = "linux")]
    Portal(linux::portal::PortalHotkeys),
    /// No backend could be started; dictation can still be triggered from
    /// the tray, the overlay, or `bars --toggle`.
    None(String),
}

impl Hotkeys {
    pub fn start(kind: HotkeyBackendKind, chord: Chord, events: Sender<HotkeyEvent>) -> Hotkeys {
        #[cfg(target_os = "linux")]
        {
            let mut errors = Vec::new();
            if matches!(kind, HotkeyBackendKind::Auto | HotkeyBackendKind::Direct) {
                match linux::evdev_hotkeys::EvdevHotkeys::spawn(chord.clone(), events.clone()) {
                    Ok(h) => return Hotkeys::Evdev(h),
                    Err(e) => {
                        log::info!("direct hotkeys unavailable: {e:#}");
                        errors.push(format!("{e:#}"));
                    }
                }
            }
            // The portal is only used when chosen (onboarding offers it), so
            // the desktop never shows shortcut dialogs the user didn't ask for.
            if kind == HotkeyBackendKind::Portal {
                match linux::portal::PortalHotkeys::spawn(chord, events) {
                    Ok(h) => return Hotkeys::Portal(h),
                    Err(e) => {
                        log::warn!("portal hotkeys unavailable: {e:#}");
                        errors.push(format!("{e:#}"));
                    }
                }
            }
            if errors.is_empty() {
                errors.push("keyboard access is needed".into());
            }
            return Hotkeys::None(errors.join("; "));
        }
        #[allow(unreachable_code)]
        {
            let _ = (kind, chord, events);
            Hotkeys::None("global hotkeys are not implemented on this platform yet".into())
        }
    }

    pub fn is_active(&self) -> bool {
        !matches!(self, Hotkeys::None(_))
    }

    /// Short name of the active backend, for display.
    pub fn backend_name(&self) -> &'static str {
        match self {
            #[cfg(target_os = "linux")]
            Hotkeys::Evdev(_) => "direct",
            #[cfg(target_os = "linux")]
            Hotkeys::Portal(_) => "desktop",
            Hotkeys::None(_) => "none",
        }
    }

    /// Whether the desktop owns the key binding (it can't be recorded here).
    pub fn desktop_managed(&self) -> bool {
        #[cfg(target_os = "linux")]
        {
            matches!(self, Hotkeys::Portal(_))
        }
        #[cfg(not(target_os = "linux"))]
        {
            false
        }
    }

    /// Open the desktop's shortcut editor (portal backend only).
    pub fn configure(&self) {
        #[cfg(target_os = "linux")]
        if let Hotkeys::Portal(p) = self {
            p.configure();
        }
    }

    pub fn error(&self) -> Option<&str> {
        match self {
            Hotkeys::None(e) => Some(e),
            #[allow(unreachable_patterns)]
            _ => None,
        }
    }

    pub fn set_chord(&self, chord: Chord) {
        match self {
            #[cfg(target_os = "linux")]
            Hotkeys::Evdev(h) => h.set_chord(chord),
            _ => {}
        }
    }

    pub fn set_capture(&self, on: bool) {
        match self {
            #[cfg(target_os = "linux")]
            Hotkeys::Evdev(h) => h.set_capture(on),
            _ => {}
        }
    }

    /// A thread-safe probe for currently held modifiers, when available.
    pub fn mods_probe(&self) -> Option<std::sync::Arc<dyn Fn() -> Mods + Send + Sync>> {
        match self {
            #[cfg(target_os = "linux")]
            Hotkeys::Evdev(h) => Some(h.mods_probe()),
            _ => None,
        }
    }

    /// Modifiers physically held right now, when the backend can tell.
    pub fn held_mods(&self) -> Option<Mods> {
        match self {
            #[cfg(target_os = "linux")]
            Hotkeys::Evdev(h) => Some(h.held_mods()),
            _ => None,
        }
    }
}

/// Inserts text into the focused application.
pub struct Injector {
    #[cfg(target_os = "linux")]
    inner: linux::inject::LinuxInjector,
}

impl Injector {
    pub fn new() -> Self {
        Injector {
            #[cfg(target_os = "linux")]
            inner: linux::inject::LinuxInjector::new(),
        }
    }

    /// Re-probe devices (e.g. after permissions were granted).
    pub fn refresh(&mut self) {
        *self = Injector::new();
    }

    pub fn can_insert(&self) -> bool {
        #[cfg(target_os = "linux")]
        {
            self.inner.has_keyboard()
        }
        #[cfg(not(target_os = "linux"))]
        {
            false
        }
    }

    pub fn insert(&mut self, text: &str, cfg: &InsertionConfig) -> anyhow::Result<()> {
        #[cfg(target_os = "linux")]
        {
            self.inner.insert(text, cfg)
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = (text, cfg);
            anyhow::bail!("text insertion is not implemented on this platform yet")
        }
    }

    pub fn copy(&self, text: &str) -> anyhow::Result<()> {
        #[cfg(target_os = "linux")]
        {
            self.inner.copy_only(text)
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = text;
            anyhow::bail!("clipboard is not implemented on this platform yet")
        }
    }
}

impl Default for Injector {
    fn default() -> Self {
        Self::new()
    }
}

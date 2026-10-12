//! Platform integration behind small, OS-neutral facades: global hotkeys,
//! text insertion and permission checks. Windows/macOS implementations slot
//! in next to `linux` without touching the controller or UI.

#[cfg(target_os = "linux")]
pub mod linux;
#[cfg(target_os = "macos")]
pub mod mac;
pub mod focus;
#[cfg(any(target_os = "windows", target_os = "macos"))]
mod native_doctor;
#[cfg(any(target_os = "windows", target_os = "macos"))]
pub mod native_inject;
pub mod paste;
mod procs;
pub mod requirements;
#[cfg(target_os = "windows")]
pub mod win;

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
        native_doctor::report()
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
    #[cfg(target_os = "macos")]
    {
        Permissions {
            hotkeys: mac::permissions::accessibility() || mac::permissions::input_monitoring(),
            keyboard: mac::permissions::accessibility(),
            clipboard: true,
            sandboxed: false,
            fix_command: None,
            can_fix_automatically: true,
            items,
        }
    }
    #[cfg(target_os = "windows")]
    {
        Permissions {
            hotkeys: true,
            keyboard: true,
            clipboard: true,
            sandboxed: false,
            fix_command: None,
            can_fix_automatically: false,
            items,
        }
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        Permissions {
            items,
            ..Permissions::default()
        }
    }
}

/// Requests the OS-level access Bars needs (Linux: installs a udev rule
/// via polkit).
/// Asks for what's missing: everything, or just `only` where the platform
/// can ask per item (macOS shows one system prompt per permission).
pub fn fix_permissions(only: Option<requirements::ReqId>) -> anyhow::Result<()> {
    #[cfg(target_os = "linux")]
    {
        let _ = only;
        linux::permissions::install_with_pkexec()
    }
    #[cfg(target_os = "macos")]
    {
        use requirements::ReqId;
        let want = |id: ReqId| only.is_none_or(|o| o == id);
        if want(ReqId::Microphone) {
            mac::permissions::request_microphone();
        }
        if want(ReqId::Accessibility) && !mac::permissions::accessibility() {
            // Lists Bars in Accessibility (and prompts, the first time
            // only), then opens that list so there's always a next step.
            mac::permissions::request_accessibility();
            requirements::open_url(requirements::MAC_ACCESSIBILITY_SETTINGS)?;
        }
        Ok(())
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = only;
        Ok(())
    }
}

/// Whether a shortcut backend that couldn't start could start now. Cheap
/// enough to poll: macOS grants access in System Settings, outside Bars.
pub fn hotkey_access_granted() -> bool {
    #[cfg(target_os = "macos")]
    {
        mac::permissions::accessibility() || mac::permissions::input_monitoring()
    }
    #[cfg(not(target_os = "macos"))]
    {
        false
    }
}

/// The app holding Secure Keyboard Entry (macOS), which keeps every other
/// app, Bars included, from seeing keys. None when off, or held by the
/// login window (the lock screen).
pub fn secure_input_owner() -> Option<String> {
    #[cfg(target_os = "macos")]
    {
        mac::secure_input::owner()
    }
    #[cfg(not(target_os = "macos"))]
    {
        None
    }
}

/// Starts what reading the focused field needs, ahead of the first
/// dictation (Linux: following focus over the accessibility bus).
pub fn prepare_focus_reading() {
    #[cfg(target_os = "linux")]
    linux::atspi::warm_up();
}

/// The active global-hotkey backend.
pub enum Hotkeys {
    #[cfg(target_os = "linux")]
    Evdev(linux::evdev_hotkeys::EvdevHotkeys),
    #[cfg(target_os = "linux")]
    Portal(linux::portal::PortalHotkeys),
    #[cfg(target_os = "windows")]
    Windows(win::hotkeys::WinHotkeys),
    #[cfg(target_os = "macos")]
    Mac(mac::hotkeys::MacHotkeys),
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
        #[cfg(target_os = "windows")]
        {
            let _ = kind;
            return match win::hotkeys::WinHotkeys::spawn(chord, events) {
                Ok(h) => Hotkeys::Windows(h),
                Err(e) => Hotkeys::None(format!("{e:#}")),
            };
        }
        #[cfg(target_os = "macos")]
        {
            let _ = kind;
            return match mac::hotkeys::MacHotkeys::spawn(chord, events) {
                Ok(h) => Hotkeys::Mac(h),
                Err(e) => Hotkeys::None(format!("{e:#}")),
            };
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
            #[cfg(target_os = "windows")]
            Hotkeys::Windows(_) => "direct",
            #[cfg(target_os = "macos")]
            Hotkeys::Mac(_) => "direct",
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
            #[cfg(target_os = "windows")]
            Hotkeys::Windows(h) => h.set_chord(chord),
            #[cfg(target_os = "macos")]
            Hotkeys::Mac(h) => h.set_chord(chord),
            _ => {
                let _ = chord;
            }
        }
    }

    /// Tells the backend a dictation is recording (and whether hands-free),
    /// so Escape cancels it without also reaching the app.
    pub fn set_recording(&self, on: bool, hands_free: bool) {
        match self {
            #[cfg(target_os = "linux")]
            Hotkeys::Evdev(h) => h.set_recording(on, hands_free),
            #[cfg(target_os = "windows")]
            Hotkeys::Windows(h) => h.set_recording(on),
            #[cfg(target_os = "macos")]
            Hotkeys::Mac(h) => h.set_recording(on),
            _ => {
                let _ = (on, hands_free);
            }
        }
    }

    pub fn set_capture(&self, on: bool) {
        match self {
            #[cfg(target_os = "linux")]
            Hotkeys::Evdev(h) => h.set_capture(on),
            #[cfg(target_os = "windows")]
            Hotkeys::Windows(h) => h.set_capture(on),
            #[cfg(target_os = "macos")]
            Hotkeys::Mac(h) => h.set_capture(on),
            _ => {
                let _ = on;
            }
        }
    }

    /// A thread-safe probe for currently held modifiers, when available.
    pub fn mods_probe(&self) -> Option<std::sync::Arc<dyn Fn() -> Mods + Send + Sync>> {
        match self {
            #[cfg(target_os = "linux")]
            Hotkeys::Evdev(h) => Some(h.mods_probe()),
            #[cfg(target_os = "windows")]
            Hotkeys::Windows(h) => Some(h.mods_probe()),
            #[cfg(target_os = "macos")]
            Hotkeys::Mac(h) => Some(h.mods_probe()),
            _ => None,
        }
    }

    /// Modifiers physically held right now, when the backend can tell.
    pub fn held_mods(&self) -> Option<Mods> {
        self.mods_probe().map(|f| f())
    }
}

/// How an insertion went.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Inserted {
    /// Pasted, and an app read the text.
    Landed,
    /// Typed, or pasted where reads can't be seen.
    Sent,
    /// Pasted, but nothing read it: no text field had focus.
    NotRead,
}

/// Inserts text into the focused application.
pub struct Injector {
    #[cfg(target_os = "linux")]
    inner: linux::inject::LinuxInjector,
    #[cfg(any(target_os = "windows", target_os = "macos"))]
    inner: native_inject::NativeInjector,
}

impl Injector {
    pub fn new() -> Self {
        Injector {
            #[cfg(target_os = "linux")]
            inner: linux::inject::LinuxInjector::new(),
            #[cfg(any(target_os = "windows", target_os = "macos"))]
            inner: native_inject::NativeInjector::new(),
        }
    }

    /// Re-probe devices (e.g. after permissions were granted).
    pub fn refresh(&mut self) {
        *self = Injector::new();
    }

    pub fn can_insert(&self) -> bool {
        #[cfg(any(target_os = "linux", target_os = "windows", target_os = "macos"))]
        {
            self.inner.has_keyboard()
        }
        #[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
        {
            false
        }
    }

    /// Called when dictation starts, so insertion goes back to that app.
    pub fn remember_target(&mut self) {
        #[cfg(any(target_os = "windows", target_os = "macos"))]
        self.inner.remember_target();
    }

    /// Inserts `pieces` one after another (one piece, unless a long text
    /// was split for a terminal agent). `on_unread` is called as soon as a
    /// paste is known not to have landed.
    pub fn insert(&mut self, pieces: &[String], cfg: &InsertionConfig, on_unread: &mut dyn FnMut()) -> anyhow::Result<Inserted> {
        #[cfg(any(target_os = "linux", target_os = "windows", target_os = "macos"))]
        {
            self.inner.insert(pieces, cfg, on_unread)
        }
        #[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
        {
            let _ = (pieces, cfg, on_unread);
            anyhow::bail!("text insertion is not implemented on this platform yet")
        }
    }

    /// Whether insertion must wait for the user to let go of the shortcut,
    /// so its keys don't combine with ours. macOS pastes with the Command
    /// flag set on the key itself, which held keys can't change.
    pub fn waits_for_released_keys(&self, cfg: &InsertionConfig) -> bool {
        #[cfg(target_os = "macos")]
        {
            cfg.method == crate::config::InsertMethod::Type
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = cfg;
            true
        }
    }

    pub fn copy(&self, text: &str) -> anyhow::Result<()> {
        #[cfg(any(target_os = "linux", target_os = "windows", target_os = "macos"))]
        {
            self.inner.copy_only(text)
        }
        #[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
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

//! What the app needs from the OS, per platform, with how to check and fix
//! each item. The onboarding UI renders this list; adding a platform means
//! adding its entries here, not new UI.

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum ReqId {
    /// A microphone is present (and on macOS/Windows, allowed).
    Microphone,
    /// Linux: read keyboards + create a virtual keyboard (udev rule).
    KeyboardAccess,
    /// macOS: Accessibility (posting key events to type/paste).
    Accessibility,
    /// macOS: Input Monitoring (observing the global shortcut).
    InputMonitoring,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ReqStatus {
    Granted,
    Missing,
    /// Can't be checked from here (e.g. not implemented, or sandboxed).
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FixAction {
    /// Ask the OS for elevated rights and fix it automatically
    /// (Linux: polkit; also from inside Flatpak via the host).
    Elevate,
    /// Open a system settings pane (macOS privacy, Windows privacy).
    OpenSettings { label: String, url: String },
    /// A command the user can run themselves.
    Command(String),
    /// Use a permission-free alternative (desktop portals) instead.
    UseAlternative { label: String, detail: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Requirement {
    pub id: ReqId,
    pub title: String,
    /// Why the app needs it, in plain words.
    pub why: String,
    pub status: ReqStatus,
    /// Shown when granted, e.g. which backend is in use.
    pub granted_note: Option<String>,
    pub actions: Vec<FixAction>,
}

impl Requirement {
    pub fn ok(&self) -> bool {
        self.status != ReqStatus::Missing
    }
}

/// Context the checks need from the running app.
#[derive(Clone, Debug, Default)]
pub struct Context {
    /// The hotkey backend currently running ("direct", "desktop", "none").
    pub hotkey_backend: &'static str,
    /// The user chose the portal alternative on purpose.
    pub prefers_portal: bool,
}

pub fn check(ctx: &Context) -> Vec<Requirement> {
    let mut out = vec![microphone()];
    #[cfg(target_os = "linux")]
    out.push(linux_keyboard(ctx));
    #[cfg(target_os = "macos")]
    out.extend(macos());
    let _ = ctx;
    out
}

fn microphone() -> Requirement {
    let devices = crate::audio::list_input_devices();
    #[cfg(target_os = "macos")]
    let actions = vec![FixAction::OpenSettings {
        label: "Open Microphone settings".into(),
        url: "x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone".into(),
    }];
    #[cfg(target_os = "windows")]
    let actions = vec![FixAction::OpenSettings {
        label: "Open microphone privacy settings".into(),
        url: "ms-settings:privacy-microphone".into(),
    }];
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let actions = Vec::new();
    Requirement {
        id: ReqId::Microphone,
        title: "Microphone".into(),
        why: "Bars listens only while you hold the shortcut. Audio is transcribed on this computer and never saved."
            .into(),
        status: if devices.is_empty() {
            ReqStatus::Missing
        } else {
            ReqStatus::Granted
        },
        granted_note: devices
            .iter()
            .find(|d| d.is_default)
            .or(devices.first())
            .map(|d| d.name.clone()),
        actions,
    }
}

#[cfg(target_os = "linux")]
fn linux_keyboard(ctx: &Context) -> Requirement {
    use super::linux;
    // BARS_DEV_NO_ACCESS=1 previews the onboarding as a new user sees it.
    let direct = std::env::var_os("BARS_DEV_NO_ACCESS").is_none()
        && linux::evdev_hotkeys::can_read_keyboards()
        && linux::permissions::can_open_uinput();
    let using_portal = ctx.hotkey_backend == "desktop";
    let status = if direct || (using_portal && ctx.prefers_portal) {
        ReqStatus::Granted
    } else {
        ReqStatus::Missing
    };
    let mut actions = Vec::new();
    if linux::permissions::can_elevate() {
        actions.push(FixAction::Elevate);
    } else {
        actions.push(FixAction::Command(linux::permissions::manual_command()));
    }
    actions.push(FixAction::UseAlternative {
        label: "Use desktop shortcuts instead".into(),
        detail: "No system changes. Your desktop asks once to register the shortcut and to let Bars type. Shortcuts must include a regular key (e.g. Ctrl+Super+Space).".into(),
    });
    Requirement {
        id: ReqId::KeyboardAccess,
        title: "Keyboard access".into(),
        why: "Lets Bars notice your shortcut in every app and type the result, like a keyboard. Installs a small system rule that applies only to you while you're logged in.".into(),
        status,
        granted_note: Some(if direct {
            "Direct access: any shortcut, including Ctrl + Super".into()
        } else {
            "Using your desktop's shortcut and remote-control portals".into()
        }),
        actions,
    }
}

#[cfg(target_os = "macos")]
fn macos() -> Vec<Requirement> {
    // Checks land with the macOS backend (AXIsProcessTrusted,
    // IOHIDCheckAccess); until then the UI shows these as "check settings".
    vec![
        Requirement {
            id: ReqId::Accessibility,
            title: "Accessibility".into(),
            why: "Needed to paste or type your words into the app you're using.".into(),
            status: ReqStatus::Unknown,
            granted_note: None,
            actions: vec![FixAction::OpenSettings {
                label: "Open Accessibility settings".into(),
                url: "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility".into(),
            }],
        },
        Requirement {
            id: ReqId::InputMonitoring,
            title: "Input Monitoring".into(),
            why: "Needed to notice your dictation shortcut while other apps are focused.".into(),
            status: ReqStatus::Unknown,
            granted_note: None,
            actions: vec![FixAction::OpenSettings {
                label: "Open Input Monitoring settings".into(),
                url: "x-apple.systempreferences:com.apple.preference.security?Privacy_ListenEvent".into(),
            }],
        },
    ]
}

/// Opens a settings URL with the platform's handler.
pub fn open_url(url: &str) -> anyhow::Result<()> {
    #[cfg(target_os = "macos")]
    let mut cmd = std::process::Command::new("open");
    #[cfg(target_os = "windows")]
    let mut cmd = {
        let mut c = std::process::Command::new("cmd");
        c.args(["/C", "start", ""]);
        c
    };
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let mut cmd = std::process::Command::new("xdg-open");
    cmd.arg(url).spawn()?;
    Ok(())
}

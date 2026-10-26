//! `bars --doctor` on Windows and macOS: a plain-text report of what this
//! machine supports and what Bars is allowed to do.

use std::fmt::Write as _;

use super::requirements::{self, ReqStatus};
use crate::config::Config;

fn mark(ok: bool) -> &'static str {
    if ok { "yes" } else { "no " }
}

fn os_version() -> String {
    #[cfg(target_os = "windows")]
    {
        super::win::permissions::os_version()
    }
    #[cfg(target_os = "macos")]
    {
        let v = std::process::Command::new("sw_vers")
            .arg("-productVersion")
            .output()
            .ok()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_default();
        format!("macOS {v}")
    }
}

pub fn report() -> String {
    let mut out = String::new();
    let _ = writeln!(out, "Bars {} diagnostics", env!("CARGO_PKG_VERSION"));
    let _ = writeln!(out, "system: {} · {}", os_version(), std::env::consts::ARCH);
    if let Ok(exe) = std::env::current_exe() {
        let _ = writeln!(out, "app: {}", exe.display());
    }

    let config_path = crate::paths::config_file();
    let cfg = Config::load_or_default(&config_path);
    let _ = writeln!(out, "\nSettings ({})", config_path.display());
    let shortcuts: Vec<String> = cfg.hotkey.chords.iter().map(|c| c.to_string()).collect();
    let _ = writeln!(out, "  shortcuts: {} ({:?})", shortcuts.join(", "), cfg.hotkey.activation);
    let _ = writeln!(out, "  insertion: {:?}, restore clipboard: {}", cfg.insertion.method, cfg.insertion.restore_clipboard);
    let _ = writeln!(out, "  model: {}", cfg.model.id);

    let _ = writeln!(out, "\nPermissions");
    for r in requirements::check(&requirements::Context::default()) {
        let state = match r.status {
            ReqStatus::Granted => "yes",
            ReqStatus::Missing => "no ",
            ReqStatus::Unknown => " ? ",
        };
        let note = match (&r.status, &r.granted_note) {
            (ReqStatus::Granted, Some(n)) => format!(" · {n}"),
            _ => String::new(),
        };
        let _ = writeln!(out, "  [{state}] {}{note}", r.title);
        if r.status == ReqStatus::Missing {
            for a in &r.actions {
                if let requirements::FixAction::OpenSettings { label, url } = a {
                    let _ = writeln!(out, "        {label}: {url}");
                }
            }
        }
    }

    let _ = writeln!(out, "\nInput");
    #[cfg(target_os = "windows")]
    let _ = writeln!(out, "  [yes] global shortcut (low-level keyboard hook, no permission needed)");
    #[cfg(target_os = "macos")]
    let _ = writeln!(
        out,
        "  [{}] global shortcut (event tap; needs {})",
        mark(super::mac::permissions::control() || super::mac::permissions::input_monitoring()),
        super::mac::permissions::accessibility_pane()
    );
    #[cfg(target_os = "macos")]
    let _ = writeln!(out, "        {}", super::mac::permissions::summary());
    #[cfg(target_os = "macos")]
    let _ = writeln!(
        out,
        "  [{}] clipboard restore (reading it: {:?})",
        mark(super::clipboard_restore_allowed()),
        super::mac::paste::read_access()
    );
    let keys = super::native_inject::NativeInjector::new();
    let _ = writeln!(out, "  [{}] keyboard simulation (paste/type)", mark(keys.has_keyboard()));
    let clipboard = arboard::Clipboard::new().is_ok();
    let _ = writeln!(out, "  [{}] clipboard", mark(clipboard));

    let _ = writeln!(out, "\nAudio");
    let devices = crate::audio::list_input_devices();
    if devices.is_empty() {
        let _ = writeln!(out, "  (no microphones found)");
    }
    for d in devices {
        let _ = writeln!(out, "  {} {}", if d.is_default { "*" } else { " " }, d.name);
    }

    let _ = writeln!(out, "\nModels ({})", crate::paths::models_dir().display());
    for m in crate::models::CATALOG {
        let _ = writeln!(out, "  [{}] {} ({})", mark(m.is_installed()), m.name, m.id);
    }
    out
}

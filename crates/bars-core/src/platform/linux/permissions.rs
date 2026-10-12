//! One-time device access setup.
//!
//! The direct hotkey backend reads keyboards and the injector writes to
//! `/dev/uinput`. Rather than adding the user to the `input` group (which on
//! image-based distros means editing /etc/group), we install a udev rule
//! that tags these devices `uaccess`: systemd-logind then grants access to
//! whoever is logged in at the physical seat, and revokes it on logout.

use std::process::Command;

pub const RULE_PATH: &str = "/etc/udev/rules.d/70-bars.rules";

pub const RULE: &str = r#"# Bars voice dictation: let the active seat user observe keyboards (global
# push-to-talk hotkey) and create a virtual keyboard (to type transcripts).
KERNEL=="uinput", SUBSYSTEM=="misc", TAG+="uaccess", OPTIONS+="static_node=uinput"
SUBSYSTEM=="input", KERNEL=="event*", ENV{ID_INPUT_KEYBOARD}=="1", TAG+="uaccess"
"#;

/// Shell snippet that installs the rule and applies it immediately.
pub fn install_script() -> String {
    format!(
        "cat > {RULE_PATH} <<'EOF'\n{RULE}EOF\nudevadm control --reload-rules && udevadm trigger --action=change --subsystem-match=input --subsystem-match=misc"
    )
}

/// The command a user can paste into a terminal.
pub fn manual_command() -> String {
    format!("sudo sh -c '{}'", install_script().replace('\'', r"'\''"))
}

pub fn can_open_uinput() -> bool {
    std::fs::OpenOptions::new()
        .write(true)
        .open("/dev/uinput")
        .is_ok()
}

pub fn rule_installed() -> bool {
    std::path::Path::new(RULE_PATH).exists()
}

/// Whether we can ask for admin rights with a graphical prompt: polkit
/// directly, or from inside Flatpak through the host (needs the
/// `org.freedesktop.Flatpak` talk permission).
pub fn can_elevate() -> bool {
    static CAN: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *CAN.get_or_init(|| {
        let ok = |cmd: &mut Command| cmd.output().map(|o| o.status.success()).unwrap_or(false);
        if crate::paths::is_flatpak() {
            ok(Command::new("flatpak-spawn").args(["--host", "sh", "-c", "command -v pkexec"]))
        } else {
            ok(Command::new("sh").args(["-c", "command -v pkexec"]))
        }
    })
}

/// Installs the rule through polkit (graphical password prompt).
pub fn install_with_pkexec() -> anyhow::Result<()> {
    let script = install_script();
    let status = if crate::paths::is_flatpak() {
        Command::new("flatpak-spawn")
            .args(["--host", "pkexec", "sh", "-c", &script])
            .status()?
    } else {
        Command::new("pkexec").args(["sh", "-c", &script]).status()?
    };
    if !status.success() {
        anyhow::bail!("authorization was cancelled or failed ({status})");
    }
    // Give logind a moment to apply the new ACLs.
    std::thread::sleep(std::time::Duration::from_millis(800));
    Ok(())
}

//! Launch at login. Native installs write an XDG autostart entry; inside
//! Flatpak the Background portal does it on our behalf.

use bars_core::paths;

const FILE: &str = "xyz.hink.Bars.desktop";

pub fn sync(enabled: bool) {
    if paths::is_flatpak() {
        std::thread::spawn(move || {
            if let Err(e) = smol::block_on(request_portal(enabled)) {
                log::warn!("background portal: {e:#}");
            }
        });
        return;
    }
    let Some(dir) = dirs_autostart() else { return };
    let path = dir.join(FILE);
    let exe_path = std::env::current_exe().ok();
    // Never register a development build running out of a cargo target dir.
    if exe_path
        .as_ref()
        .is_some_and(|p| p.components().any(|c| c.as_os_str() == "target"))
    {
        log::debug!("autostart: skipping for development build");
        return;
    }
    if enabled {
        let exe = exe_path
            .and_then(|p| p.to_str().map(str::to_string))
            .unwrap_or_else(|| "bars".into());
        // AppImage: point at the image, not the mounted binary.
        let exe = std::env::var("APPIMAGE").unwrap_or(exe);
        let entry = format!(
            "[Desktop Entry]\nType=Application\nName=Bars\nComment=Voice dictation\nExec=\"{exe}\" --background\nIcon=xyz.hink.Bars\nX-GNOME-Autostart-enabled=true\nNoDisplay=true\n"
        );
        let _ = std::fs::create_dir_all(&dir);
        if std::fs::read_to_string(&path).ok().as_deref() != Some(entry.as_str()) {
            if let Err(e) = std::fs::write(&path, entry) {
                log::warn!("autostart: {e}");
            }
        }
    } else {
        let _ = std::fs::remove_file(path);
    }
}

fn dirs_autostart() -> Option<std::path::PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".config")))
        .map(|c| c.join("autostart"))
}

#[cfg(target_os = "linux")]
async fn request_portal(enabled: bool) -> anyhow::Result<()> {
    use ashpd::desktop::background::Background;
    Background::request()
        .reason("Start Bars at login so dictation is always one shortcut away")
        .auto_start(enabled)
        .command(&["bars", "--background"])
        .dbus_activatable(false)
        .send()
        .await?
        .response()?;
    Ok(())
}

#[cfg(not(target_os = "linux"))]
async fn request_portal(_: bool) -> anyhow::Result<()> {
    Ok(())
}

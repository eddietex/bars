//! Filesystem locations. Inside Flatpak these resolve to `~/.var/app/<id>/...`
//! automatically because the sandbox rewrites the XDG variables.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

pub const APP_ID: &str = "xyz.hink.Bars";
pub const APP_DIR: &str = "bars";

fn ensure(dir: PathBuf) -> PathBuf {
    let _ = std::fs::create_dir_all(&dir);
    dir
}

pub fn config_dir() -> PathBuf {
    ensure(
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(APP_DIR),
    )
}

pub fn data_dir() -> PathBuf {
    ensure(
        dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(APP_DIR),
    )
}

pub fn models_dir() -> PathBuf {
    ensure(data_dir().join("models"))
}

pub fn config_file() -> PathBuf {
    config_dir().join("config.toml")
}

pub fn history_file() -> PathBuf {
    data_dir().join("history.jsonl")
}

pub fn is_flatpak() -> bool {
    Path::new("/.flatpak-info").exists()
}

/// This program's path, as it was when first asked; `main` asks at startup.
/// Once an update has replaced the binary, Linux names the running one
/// "… (deleted)", so asking then would give the wrong path.
pub fn exe() -> Option<&'static Path> {
    static EXE: OnceLock<Option<PathBuf>> = OnceLock::new();
    EXE.get_or_init(|| std::env::current_exe().ok()).as_deref()
}

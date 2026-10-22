//! Updates. Bars only tells the user that a newer version is published and
//! installs it when they ask: on macOS and Windows it replaces itself, on
//! Linux (and builds that can't replace themselves) the release page opens.
//!
//! What's published is read from a small manifest on GitHub Releases,
//! `latest.json`, attached to every release by the release workflow. A
//! draft release is never "latest", so publishing a release is what offers
//! it. Downloads are checked against Bars' own signing key (minisign), not
//! the code signature.

use anyhow::{Context, anyhow};
use semver::Version;
use serde::Deserialize;

/// The newest published release's manifest.
const MANIFEST: &str = "https://github.com/hink/bars/releases/latest/download/latest.json";
/// Where to download by hand.
pub const RELEASE_PAGE: &str = "https://github.com/hink/bars/releases/latest";
/// Public half of the key updates are signed with: the contents of the
/// `.pub` file from `cargo packager signer generate` (minisign key
/// EF7F31A8EDF9A6B4). Empty: updates can't be verified, so none are
/// installed (the release page opens instead).
const PUBLIC_KEY: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IEVGN0YzMUE4RURGOUE2QjQKUldTMHB2bnRxREYvN3dYcThOUVZqTjVvK2pOYUhBczQ0d2tQNjlFTU1lekNGUUpFYS8wd0MyM0sK";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Release {
    pub version: String,
    pub notes: Option<String>,
}

#[derive(Deserialize)]
struct Manifest {
    version: String,
    notes: Option<String>,
}

/// `BARS_DEV_UPDATE_MANIFEST` points at another manifest, for testing.
fn manifest_url() -> String {
    std::env::var("BARS_DEV_UPDATE_MANIFEST").unwrap_or_else(|_| MANIFEST.into())
}

/// The latest release, if it's newer than this build.
pub fn check() -> anyhow::Result<Option<Release>> {
    let mut resp = crate::models::http_agent()
        .get(&manifest_url())
        .call()
        .context("fetching the release manifest")?;
    let body = resp.body_mut().read_to_string().context("reading the release manifest")?;
    let manifest: Manifest = serde_json::from_str(&body).context("reading the release manifest")?;
    Ok(newer(env!("CARGO_PKG_VERSION"), manifest))
}

fn newer(current: &str, manifest: Manifest) -> Option<Release> {
    let latest = Version::parse(manifest.version.trim_start_matches('v')).ok()?;
    let current = Version::parse(current).ok()?;
    (latest > current).then(|| Release {
        version: latest.to_string(),
        notes: manifest.notes.filter(|n| !n.trim().is_empty()),
    })
}

/// Whether this copy can install an update itself: an installed app with a
/// key to verify updates. A build run from source can't (its folder would
/// be replaced), and neither can Linux, where packages update elsewhere.
pub fn can_install() -> bool {
    !PUBLIC_KEY.is_empty() && installed_app()
}

#[cfg(target_os = "macos")]
fn installed_app() -> bool {
    // .../Bars.app/Contents/MacOS/bars
    std::env::current_exe().is_ok_and(|exe| {
        exe.parent().is_some_and(|dir| dir.ends_with("Contents/MacOS"))
            && exe.ancestors().nth(3).is_some_and(|app| app.extension().is_some_and(|e| e == "app"))
    })
}

#[cfg(target_os = "windows")]
fn installed_app() -> bool {
    // The installer puts its uninstaller next to bars.exe.
    std::env::current_exe().is_ok_and(|exe| exe.with_file_name("uninstall.exe").exists())
}

#[cfg(target_os = "linux")]
fn installed_app() -> bool {
    false
}

/// Downloads, verifies and installs the latest release. Windows: the
/// installer takes over and this process exits. macOS: the app is replaced
/// on disk; call `relaunch_after_exit` and quit.
#[cfg(any(target_os = "macos", target_os = "windows"))]
pub fn install() -> anyhow::Result<()> {
    use cargo_packager_updater::{Config, WindowsConfig, WindowsUpdateInstallMode};
    if !can_install() {
        return Err(anyhow!("this copy of Bars can't update itself"));
    }
    let config = Config {
        endpoints: vec![manifest_url().parse()?],
        pubkey: PUBLIC_KEY.into(),
        windows: Some(WindowsConfig {
            installer_args: None,
            install_mode: Some(WindowsUpdateInstallMode::Passive),
        }),
    };
    let current = Version::parse(env!("CARGO_PKG_VERSION"))?;
    let update = cargo_packager_updater::check_update(current, config)?.ok_or_else(|| anyhow!("no newer version"))?;
    log::info!("installing Bars {}", update.version);
    update.download_and_install()?;
    Ok(())
}

#[cfg(target_os = "linux")]
pub fn install() -> anyhow::Result<()> {
    Err(anyhow!("this copy of Bars can't update itself"))
}

/// macOS: opens the app (replaced, perhaps) once this process has exited.
/// One that hasn't after 10 s is stuck (in the audio system, say) and is
/// killed.
#[cfg(target_os = "macos")]
pub fn relaunch_after_exit() -> anyhow::Result<()> {
    let exe = std::env::current_exe()?;
    let app = exe
        .ancestors()
        .nth(3)
        .filter(|p| p.extension().is_some_and(|e| e == "app"))
        .ok_or_else(|| anyhow!("not inside an app"))?;
    let script = r#"i=0; while kill -0 "$1" 2>/dev/null; do i=$((i+1)); [ "$i" -gt 50 ] && kill -9 "$1"; sleep 0.2; done; open "$2""#;
    std::process::Command::new("/bin/sh")
        .args(["-c", script, "sh"])
        .arg(std::process::id().to_string())
        .arg(app)
        .spawn()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(version: &str) -> Manifest {
        Manifest { version: version.into(), notes: Some(" ".into()) }
    }

    #[test]
    fn offers_only_newer_versions() {
        assert_eq!(newer("0.3.0", manifest("0.3.1")).map(|r| r.version), Some("0.3.1".into()));
        assert_eq!(newer("0.3.0", manifest("v0.4.0")).map(|r| r.version), Some("0.4.0".into()));
        assert_eq!(newer("0.3.0", manifest("0.3.0")), None);
        assert_eq!(newer("0.3.1", manifest("0.3.0")), None);
        assert_eq!(newer("0.3.0", manifest("not a version")), None);
        // Blank notes are dropped.
        assert_eq!(newer("0.3.0", manifest("0.3.1")).unwrap().notes, None);
    }

    #[test]
    fn reads_the_release_manifest() {
        let m: Manifest = serde_json::from_str(
            r#"{"version":"0.3.1","notes":"Fixes","pub_date":"2026-10-12T00:00:00Z",
                "platforms":{"macos-aarch64":{"url":"https://example.com/a.tar.gz","signature":"x","format":"app"}}}"#,
        )
        .unwrap();
        assert_eq!(newer("0.3.0", m), Some(Release { version: "0.3.1".into(), notes: Some("Fixes".into()) }));
    }
}

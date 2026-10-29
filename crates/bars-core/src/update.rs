//! Updates. Bars only tells the user that a newer version is published and
//! installs it when they ask: on macOS, Windows and Linux (the tarball's
//! `bin/bars`, wherever it was put) it replaces itself; for the Flatpak and
//! builds that can't replace themselves the release page opens.
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

/// This program's path, read once: after an update replaced it, Linux names
/// the running one "… (deleted)".
#[cfg(target_os = "linux")]
static EXE: std::sync::LazyLock<Option<std::path::PathBuf>> = std::sync::LazyLock::new(|| std::env::current_exe().ok());

/// Whether this copy can install an update itself: an installed app with a
/// key to verify updates. A build run from source can't (its folder would
/// be replaced), and neither can the Flatpak, which Flatpak updates.
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
    // A package installed where Bars can't write (/usr/bin) is updated by
    // its package manager.
    !std::path::Path::new("/.flatpak-info").exists()
        && EXE.as_deref().is_some_and(|exe| {
            !exe.ancestors().any(|dir| dir.ends_with("target"))
                && exe.parent().is_some_and(|dir| nix::unistd::access(dir, nix::unistd::AccessFlags::W_OK).is_ok())
        })
}

/// Downloads, verifies and installs the latest release. Windows: the
/// installer takes over and this process exits. macOS and Linux: Bars is
/// replaced on disk; to restart into it, call `relaunch_after_exit` and quit.
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

/// Linux: the tarball's `bin/bars` replaces this program. The tarballs are
/// listed in the manifest under `tarballs`, not `platforms`, whose formats
/// the macOS and Windows updater checks.
#[cfg(target_os = "linux")]
pub fn install() -> anyhow::Result<()> {
    use std::collections::HashMap;
    use std::io::Read;
    use std::os::unix::fs::OpenOptionsExt;

    #[derive(Deserialize)]
    struct Tarball {
        url: String,
        signature: String,
    }
    #[derive(Deserialize)]
    struct LinuxManifest {
        version: String,
        #[serde(default)]
        tarballs: HashMap<String, Tarball>,
    }

    if !can_install() {
        return Err(anyhow!("this copy of Bars can't update itself"));
    }
    let exe = EXE.clone().ok_or_else(|| anyhow!("can't find this program"))?;
    let agent = crate::models::http_agent();
    let body = agent.get(&manifest_url()).call()?.body_mut().read_to_string().context("reading the release manifest")?;
    let mut manifest: LinuxManifest = serde_json::from_str(&body).context("reading the release manifest")?;
    let latest = Version::parse(manifest.version.trim_start_matches('v'))?;
    if latest <= Version::parse(env!("CARGO_PKG_VERSION"))? {
        return Err(anyhow!("no newer version"));
    }
    let target = format!("linux-{}", std::env::consts::ARCH);
    let tarball = manifest.tarballs.remove(&target).ok_or_else(|| anyhow!("no update for {target}"))?;
    log::info!("installing Bars {latest}");
    let mut archive = Vec::new();
    agent
        .get(&tarball.url)
        .call()?
        .body_mut()
        .with_config()
        .limit(u64::MAX)
        .reader()
        .read_to_end(&mut archive)
        .context("downloading the update")?;
    verify(&archive, &tarball.signature, PUBLIC_KEY).context("checking the update's signature")?;

    let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(archive.as_slice()));
    for entry in tar.entries()? {
        let mut entry = entry?;
        if !entry.path()?.ends_with("bin/bars") {
            continue;
        }
        // Written next to Bars and renamed over it, so it's never half there.
        let tmp = exe.with_file_name(".bars-update");
        let result = (|| {
            let mut file = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o755).open(&tmp)?;
            std::io::copy(&mut entry, &mut file)?;
            file.sync_all()?;
            std::fs::rename(&tmp, &exe)
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
        return result.context("replacing Bars");
    }
    Err(anyhow!("the update has no bin/bars"))
}

/// Checks `data` against a signature from `cargo packager signer sign`, as
/// the macOS and Windows updater does: both it and the key are base64 of
/// minisign's text files.
#[cfg(target_os = "linux")]
fn verify(data: &[u8], signature: &str, key: &str) -> anyhow::Result<()> {
    use base64::Engine;
    let text = |b64: &str| -> anyhow::Result<String> {
        Ok(String::from_utf8(base64::engine::general_purpose::STANDARD.decode(b64.trim())?)?)
    };
    let key = minisign_verify::PublicKey::decode(&text(key)?)?;
    let signature = minisign_verify::Signature::decode(&text(signature)?)?;
    key.verify(data, &signature, true)?;
    Ok(())
}

/// macOS and Linux: starts Bars (replaced, perhaps) again once this process
/// has exited. One that hasn't after 10 s is stuck (in the audio system,
/// say) and is killed.
#[cfg(any(target_os = "macos", target_os = "linux"))]
pub fn relaunch_after_exit() -> anyhow::Result<()> {
    #[cfg(target_os = "macos")]
    let (start, target) = {
        let exe = std::env::current_exe()?;
        let app = exe
            .ancestors()
            .nth(3)
            .filter(|p| p.extension().is_some_and(|e| e == "app"))
            .ok_or_else(|| anyhow!("not inside an app"))?
            .to_path_buf();
        (r#"open "$2""#, app)
    };
    #[cfg(target_os = "linux")]
    let (start, target) = (r#"exec "$2""#, EXE.clone().ok_or_else(|| anyhow!("can't find this program"))?);
    let script = format!(r#"i=0; while kill -0 "$1" 2>/dev/null; do i=$((i+1)); [ "$i" -gt 50 ] && kill -9 "$1"; sleep 0.2; done; {start}"#);
    std::process::Command::new("/bin/sh")
        .args(["-c", &script, "sh"])
        .arg(std::process::id().to_string())
        .arg(target)
        .stdin(std::process::Stdio::null())
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

    /// Signed by `cargo packager signer sign`, as the release workflow does,
    /// with a throwaway key.
    #[cfg(target_os = "linux")]
    #[test]
    fn checks_update_signatures() {
        const KEY: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IDMwNjZCNDUzOERDODcxMUMKUldRY2NjaU5VN1JtTUl4SGR1QjU0eno2MkZTOWNyWWxSdklTN1Y5eE5ORzVFSXFPUVkvL2pxNDgK";
        const SIGNATURE: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IHNpZ25hdHVyZSBmcm9tIGNhcmdvLXBhY2thZ2VyIHNlY3JldCBrZXkKUlVRY2NjaU5VN1JtTUJkWiswMThBSUE1eEVtQjlhWU1wUWcvb0VWZHVzSzVQVzhqeXhxOU1qbUtmcVVQb2hoYk5tVnhuZHhPUEFxdmM2a3hCMFhtd0tPalFuRXJoYXU5cHdvPQp0cnVzdGVkIGNvbW1lbnQ6IHRpbWVzdGFtcDoxNzkzMjQ4MTg4CWZpbGU6cGF5bG9hZAo4d0p5emNVeEV0eDRaU0xTY1N6VW4wNWdUcy9Zc2RHMktQQXAvcjh1Sm1KNWRubS96YXN1aDBGd3VvK0IwMFcwTnQxa1lIaHAwMnFFVVBJcG9jSDZDQT09Cg==";
        assert!(verify(b"Bars update", SIGNATURE, KEY).is_ok());
        assert!(verify(b"Bars updatf", SIGNATURE, KEY).is_err());
        assert!(verify(b"Bars update", SIGNATURE, PUBLIC_KEY).is_err());
    }
}

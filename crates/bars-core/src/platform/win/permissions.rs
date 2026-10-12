//! Windows privacy settings that affect Bars.

use windows::Win32::System::Registry::{HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, RRF_RT_REG_DWORD, RRF_RT_REG_SZ, RegGetValueW};
use windows::core::{HSTRING, PCWSTR};

fn reg_string(root: HKEY, subkey: &str, value: &str) -> Option<String> {
    let mut buf = [0u16; 128];
    let mut len = (buf.len() * 2) as u32;
    let sub = HSTRING::from(subkey);
    let val = HSTRING::from(value);
    let ok = unsafe {
        RegGetValueW(
            root,
            PCWSTR(sub.as_ptr()),
            PCWSTR(val.as_ptr()),
            RRF_RT_REG_SZ,
            None,
            Some(buf.as_mut_ptr().cast()),
            Some(&mut len),
        )
    };
    if ok.is_err() {
        return None;
    }
    let chars = (len as usize / 2).saturating_sub(1);
    Some(String::from_utf16_lossy(&buf[..chars.min(buf.len())]))
}

/// Whether "Let desktop apps access your microphone" (or the global
/// microphone switch) is turned off.
pub fn microphone_blocked() -> bool {
    const BASE: &str = r"Software\Microsoft\Windows\CurrentVersion\CapabilityAccessManager\ConsentStore\microphone";
    let deny = |v: Option<String>| v.is_some_and(|s| s.eq_ignore_ascii_case("Deny"));
    deny(reg_string(HKEY_CURRENT_USER, BASE, "Value"))
        || deny(reg_string(HKEY_CURRENT_USER, &format!(r"{BASE}\NonPackaged"), "Value"))
}

/// Whether the taskbar uses the light theme (tray icons need dark glyphs).
pub fn taskbar_is_light() -> bool {
    let sub = HSTRING::from(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize");
    let val = HSTRING::from("SystemUsesLightTheme");
    let mut data = 0u32;
    let mut len = 4u32;
    let ok = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            PCWSTR(sub.as_ptr()),
            PCWSTR(val.as_ptr()),
            RRF_RT_REG_DWORD,
            None,
            Some((&mut data as *mut u32).cast()),
            Some(&mut len),
        )
    };
    ok.is_ok() && data == 1
}

/// E.g. "Windows 11 Pro 24H2 (build 26100)".
pub fn os_version() -> String {
    const KEY: &str = r"SOFTWARE\Microsoft\Windows NT\CurrentVersion";
    let get = |v: &str| reg_string(HKEY_LOCAL_MACHINE, KEY, v);
    let build = get("CurrentBuild").unwrap_or_default();
    let mut name = get("ProductName").unwrap_or_else(|| "Windows".into());
    // ProductName still says "Windows 10" on Windows 11.
    if build.parse::<u32>().is_ok_and(|b| b >= 22000) {
        name = name.replacen("Windows 10", "Windows 11", 1);
    }
    let display = get("DisplayVersion").unwrap_or_default();
    format!("{name} {display} (build {build})")
}

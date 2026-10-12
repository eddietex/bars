//! Windows privacy settings that affect Bars.

use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RRF_RT_REG_SZ, RegGetValueW};
use windows::core::{HSTRING, PCWSTR};

fn reg_string(subkey: &str, value: &str) -> Option<String> {
    let mut buf = [0u16; 64];
    let mut len = (buf.len() * 2) as u32;
    let sub = HSTRING::from(subkey);
    let val = HSTRING::from(value);
    let ok = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
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
    deny(reg_string(BASE, "Value")) || deny(reg_string(&format!(r"{BASE}\NonPackaged"), "Value"))
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

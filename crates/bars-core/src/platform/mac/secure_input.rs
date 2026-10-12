//! Secure Keyboard Entry: while an app has it on (Terminal's menu option,
//! password managers, any focused password field), macOS hides keys from
//! every other app, so Bars' shortcut silently stops working.

use core_foundation::base::{CFType, TCFType};
use core_foundation::dictionary::{CFDictionary, CFDictionaryRef};
use core_foundation::number::CFNumber;
use core_foundation::string::CFString;
use objc2_app_kit::NSRunningApplication;

#[link(name = "Carbon", kind = "framework")]
unsafe extern "C" {
    fn IsSecureEventInputEnabled() -> bool;
}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGSessionCopyCurrentDictionary() -> CFDictionaryRef;
}

/// The name of the app holding secure input. None when it's off, or when
/// the login window holds it (the lock screen, switching users).
pub fn owner() -> Option<String> {
    if !unsafe { IsSecureEventInputEnabled() } {
        return None;
    }
    let pid = session_secure_input_pid();
    let Some(pid) = pid else { return Some("Another app".into()) };
    let app = NSRunningApplication::runningApplicationWithProcessIdentifier(pid);
    let bundle = app.as_ref().and_then(|a| a.bundleIdentifier()).map(|s| s.to_string());
    let name = app.as_ref().and_then(|a| a.localizedName()).map(|s| s.to_string()).or_else(|| process_name(pid));
    if bundle.as_deref() == Some("com.apple.loginwindow") || name.as_deref() == Some("loginwindow") {
        return None;
    }
    Some(name.unwrap_or_else(|| "Another app".into()))
}

fn session_secure_input_pid() -> Option<i32> {
    let dict = unsafe { CGSessionCopyCurrentDictionary() };
    if dict.is_null() {
        return None;
    }
    let dict: CFDictionary<CFString, CFType> = unsafe { CFDictionary::wrap_under_create_rule(dict) };
    let value = dict.find(CFString::from_static_string("kCGSSessionSecureInputPID"))?;
    value.downcast::<CFNumber>()?.to_i32()
}

fn process_name(pid: i32) -> Option<String> {
    let mut buf = [0u8; 256];
    let n = unsafe { libc::proc_name(pid, buf.as_mut_ptr().cast(), buf.len() as u32) };
    (n > 0).then(|| String::from_utf8_lossy(&buf[..n as usize]).into_owned())
}

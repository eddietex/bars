//! macOS accessibility: the frontmost app, its focused window's title, and
//! the text around the cursor in its focused field. Every call has a short
//! timeout, so one frozen app can't stall dictation.

use std::collections::HashSet;
use std::ffi::c_void;

use core_foundation::base::{CFType, CFTypeRef, TCFType};
use core_foundation::boolean::CFBoolean;
use core_foundation::number::CFNumber;
use core_foundation::string::{CFString, CFStringRef};
use objc2_app_kit::NSRunningApplication;
use parking_lot::Mutex;

use crate::platform::focus::{CONTEXT_CHARS, Focus};
use crate::text::Surroundings;

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXUIElementCreateSystemWide() -> CFTypeRef;
    fn AXUIElementCopyAttributeValue(element: CFTypeRef, attribute: CFStringRef, value: *mut CFTypeRef) -> i32;
    fn AXUIElementCopyParameterizedAttributeValue(
        element: CFTypeRef,
        attribute: CFStringRef,
        parameter: CFTypeRef,
        value: *mut CFTypeRef,
    ) -> i32;
    fn AXUIElementSetAttributeValue(element: CFTypeRef, attribute: CFStringRef, value: CFTypeRef) -> i32;
    fn AXUIElementSetMessagingTimeout(element: CFTypeRef, seconds: f32) -> i32;
    fn AXUIElementGetPid(element: CFTypeRef, pid: *mut i32) -> i32;
    fn AXValueGetValue(value: CFTypeRef, kind: u32, out: *mut c_void) -> bool;
    fn AXValueCreate(kind: u32, value: *const c_void) -> CFTypeRef;
}

/// kAXValueCFRangeType
const AX_RANGE: u32 = 4;
/// Seconds each call to an app may take.
const TIMEOUT: f32 = 0.3;
/// Larger fields are read around the cursor only, not whole.
const READ_WHOLE_UP_TO: i64 = 50_000;

/// Chromium browsers build their accessibility tree for assistive apps
/// only when asked for the "enhanced" interface.
const CHROMIUM: &[&str] = &[
    "com.google.Chrome",
    "com.google.Chrome.beta",
    "com.google.Chrome.canary",
    "org.chromium.Chromium",
    "com.brave.Browser",
    "com.microsoft.edgemac",
    "company.thebrowser.Browser",
    "com.vivaldi.Vivaldi",
    "com.operasoftware.Opera",
];

#[repr(C)]
#[derive(Default)]
struct CFRange {
    location: isize,
    length: isize,
}

struct Element(CFType);

impl Element {
    fn system_wide() -> Option<Element> {
        let el = unsafe { AXUIElementCreateSystemWide() };
        (!el.is_null()).then(|| Element::wrap(el))
    }

    fn wrap(el: CFTypeRef) -> Element {
        let el = Element(unsafe { CFType::wrap_under_create_rule(el) });
        unsafe { AXUIElementSetMessagingTimeout(el.raw(), TIMEOUT) };
        el
    }

    fn raw(&self) -> CFTypeRef {
        self.0.as_CFTypeRef()
    }

    fn attr(&self, name: &'static str) -> Option<CFType> {
        let mut value: CFTypeRef = std::ptr::null();
        let err = unsafe { AXUIElementCopyAttributeValue(self.raw(), CFString::from_static_string(name).as_concrete_TypeRef(), &mut value) };
        (err == 0 && !value.is_null()).then(|| unsafe { CFType::wrap_under_create_rule(value) })
    }

    fn element(&self, name: &'static str) -> Option<Element> {
        let v = self.attr(name)?;
        let el = Element(v);
        unsafe { AXUIElementSetMessagingTimeout(el.raw(), TIMEOUT) };
        Some(el)
    }

    fn string(&self, name: &'static str) -> Option<String> {
        Some(self.attr(name)?.downcast::<CFString>()?.to_string())
    }

    fn int(&self, name: &'static str) -> Option<i64> {
        self.attr(name)?.downcast::<CFNumber>()?.to_i64()
    }

    fn range(&self, name: &'static str) -> Option<(i64, i64)> {
        let v = self.attr(name)?;
        let mut r = CFRange::default();
        unsafe { AXValueGetValue(v.as_CFTypeRef(), AX_RANGE, &mut r as *mut CFRange as *mut c_void) }
            .then_some((r.location as i64, r.length as i64))
    }

    /// The text in a range (UTF-16 units), without reading the whole field.
    fn string_for_range(&self, location: i64, length: i64) -> Option<String> {
        if length <= 0 {
            return Some(String::new());
        }
        let r = CFRange {
            location: location as isize,
            length: length as isize,
        };
        let param = unsafe { AXValueCreate(AX_RANGE, &r as *const CFRange as *const c_void) };
        if param.is_null() {
            return None;
        }
        let param = unsafe { CFType::wrap_under_create_rule(param) };
        let mut value: CFTypeRef = std::ptr::null();
        let name = CFString::from_static_string("AXStringForRange");
        let err = unsafe {
            AXUIElementCopyParameterizedAttributeValue(self.raw(), name.as_concrete_TypeRef(), param.as_CFTypeRef(), &mut value)
        };
        if err != 0 || value.is_null() {
            return None;
        }
        Some(unsafe { CFType::wrap_under_create_rule(value) }.downcast::<CFString>()?.to_string())
    }

    fn set_true(&self, name: &'static str) {
        let name = CFString::from_static_string(name);
        unsafe {
            AXUIElementSetAttributeValue(self.raw(), name.as_concrete_TypeRef(), CFBoolean::true_value().as_CFTypeRef());
        }
    }

    fn pid(&self) -> Option<i32> {
        let mut pid = 0;
        (unsafe { AXUIElementGetPid(self.raw(), &mut pid) } == 0).then_some(pid)
    }
}

/// Electron apps (and Chromium browsers) only expose their text to
/// assistive apps that ask; ask once per process.
fn expose_accessibility(app: &Element, pid: i32, bundle: Option<&str>) {
    static ASKED: Mutex<Option<HashSet<i32>>> = Mutex::new(None);
    if !ASKED.lock().get_or_insert_default().insert(pid) {
        return;
    }
    app.set_true("AXManualAccessibility");
    if bundle.is_some_and(|b| CHROMIUM.contains(&b)) {
        app.set_true("AXEnhancedUserInterface");
    }
}

/// Slices UTF-16 units `[start, end)` out of `s`.
fn utf16_slice(units: &[u16], start: i64, end: i64) -> String {
    let len = units.len() as i64;
    let (start, end) = (start.clamp(0, len) as usize, end.clamp(0, len) as usize);
    String::from_utf16_lossy(&units[start..end.max(start)])
}

fn surroundings(field: &Element) -> Option<Surroundings> {
    let role = field.string("AXRole");
    let subrole = field.string("AXSubrole");
    if [role.as_deref(), subrole.as_deref()].contains(&Some("AXSecureTextField")) {
        return None;
    }
    let (location, length) = field.range("AXSelectedTextRange")?;
    let (end, n) = (location + length, CONTEXT_CHARS as i64);
    let count = field.int("AXNumberOfCharacters");
    if count.is_some_and(|c| c <= READ_WHOLE_UP_TO) {
        if let Some(value) = field.string("AXValue") {
            let units: Vec<u16> = value.encode_utf16().collect();
            return Some(Surroundings {
                before: utf16_slice(&units, location - n, location),
                selected: utf16_slice(&units, location, end),
                after: utf16_slice(&units, end, end + n),
            });
        }
    }
    let start = (location - n).max(0);
    let after_len = count.map_or(n, |c| (c - end).clamp(0, n));
    Some(Surroundings {
        before: field.string_for_range(start, location - start)?,
        selected: if length > 0 { field.string("AXSelectedText").unwrap_or_default() } else { String::new() },
        after: field.string_for_range(end, after_len).unwrap_or_default(),
    })
}

pub fn read_focus(read_field: bool) -> Focus {
    let Some(system) = Element::system_wide() else { return Focus::default() };
    let Some(app) = system.element("AXFocusedApplication") else { return Focus::default() };
    let pid = app.pid();
    let running = pid.and_then(NSRunningApplication::runningApplicationWithProcessIdentifier);
    let bundle = running.as_ref().and_then(|a| a.bundleIdentifier()).map(|s| s.to_string());
    if let Some(pid) = pid {
        expose_accessibility(&app, pid, bundle.as_deref());
    }
    let window_title = app.element("AXFocusedWindow").and_then(|w| w.string("AXTitle"));
    let field = if read_field {
        app.element("AXFocusedUIElement").and_then(|f| surroundings(&f))
    } else {
        None
    };
    Focus {
        app: bundle,
        pid: pid.map(|p| p as u32),
        window_title,
        field,
        agent: None,
    }
}

//! macOS accessibility: the frontmost app, its focused window's title, and
//! the text around the cursor in its focused field. Every call has a short
//! timeout, so one frozen app can't stall dictation.

use std::collections::HashSet;
use std::ffi::c_void;

use core_foundation::array::CFArray;
use core_foundation::base::{CFType, CFTypeRef, TCFType};
use core_foundation::boolean::CFBoolean;
use core_foundation::number::CFNumber;
use core_foundation::string::{CFString, CFStringRef};
use objc2_app_kit::NSRunningApplication;
use parking_lot::Mutex;

use crate::platform::focus::Focus;
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

    fn param(&self, name: &'static str, input: &CFType) -> Option<CFType> {
        let mut value: CFTypeRef = std::ptr::null();
        let name = CFString::from_static_string(name);
        let err = unsafe {
            AXUIElementCopyParameterizedAttributeValue(self.raw(), name.as_concrete_TypeRef(), input.as_CFTypeRef(), &mut value)
        };
        (err == 0 && !value.is_null()).then(|| unsafe { CFType::wrap_under_create_rule(value) })
    }

    fn param_string(&self, name: &'static str, input: &CFType) -> Option<String> {
        Some(self.param(name, input)?.downcast::<CFString>()?.to_string())
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
        self.param_string("AXStringForRange", &unsafe { CFType::wrap_under_create_rule(param) })
    }

    /// The text between two text markers.
    fn string_between(&self, from: &CFType, to: &CFType) -> Option<String> {
        let pair = CFArray::from_CFTypes(&[from.clone(), to.clone()]);
        let range = self.param("AXTextMarkerRangeForUnorderedTextMarkers", &pair.as_CFType())?;
        self.param_string("AXStringForTextMarkerRange", &range)
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

/// A terminal's text is its screen and scrollback, and the cursor it
/// reports means nothing (Ghostty: always the start). New text is at the
/// end, so read the last `chars` characters.
fn terminal_tail(field: &Element, chars: usize) -> Option<Surroundings> {
    let count = field.int("AXNumberOfCharacters")?;
    let n = (chars as i64).min(count);
    let before = if count <= READ_WHOLE_UP_TO {
        let units: Vec<u16> = field.string("AXValue")?.encode_utf16().collect();
        utf16_slice(&units, units.len() as i64 - n, units.len() as i64)
    } else {
        field.string_for_range(count - n, n)?
    };
    Some(Surroundings { before, selected: String::new(), after: String::new() })
}

/// Rich text editors in web content (contenteditable in Firefox, for one)
/// can report no character range for the cursor, only text markers, the
/// macOS API for web text that WebKit, Gecko and Chromium all implement.
/// Reads the cursor's paragraph through those, inside editable content only.
fn marker_surroundings(field: &Element, chars: usize) -> Option<Surroundings> {
    field.attr("AXEditableAncestor")?;
    let selection = field.attr("AXSelectedTextMarkerRange")?;
    let start = field.param("AXStartTextMarkerForTextMarkerRange", &selection)?;
    let end = field.param("AXEndTextMarkerForTextMarkerRange", &selection)?;
    let paragraph = |marker: &CFType, edge: &'static str| {
        let range = field.param("AXParagraphTextMarkerRangeForTextMarker", marker)?;
        field.param(edge, &range)
    };
    let before = field.string_between(&paragraph(&start, "AXStartTextMarkerForTextMarkerRange")?, &start)?;
    let after = paragraph(&end, "AXEndTextMarkerForTextMarkerRange")
        .and_then(|para_end| field.string_between(&end, &para_end))
        .unwrap_or_default();
    let skip = before.chars().count().saturating_sub(chars);
    Some(Surroundings {
        before: before.chars().skip(skip).collect(),
        selected: field.param_string("AXStringForTextMarkerRange", &selection).unwrap_or_default(),
        after: after.chars().take(chars).collect(),
    })
}

fn surroundings(field: &Element, chars: usize, terminal: bool) -> Option<Surroundings> {
    let role = field.string("AXRole");
    let subrole = field.string("AXSubrole");
    if [role.as_deref(), subrole.as_deref()].contains(&Some("AXSecureTextField")) {
        return None;
    }
    if terminal {
        return terminal_tail(field, chars);
    }
    let Some((location, length)) = field.range("AXSelectedTextRange") else {
        return marker_surroundings(field, chars);
    };
    let (end, n) = (location + length, chars as i64);
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

pub fn read_focus(field_chars: Option<usize>) -> Focus {
    let Some(system) = Element::system_wide() else { return Focus::default() };
    let Some(app) = system.element("AXFocusedApplication") else { return Focus::default() };
    let pid = app.pid();
    let running = pid.and_then(NSRunningApplication::runningApplicationWithProcessIdentifier);
    let bundle = running.as_ref().and_then(|a| a.bundleIdentifier()).map(|s| s.to_string());
    if let Some(pid) = pid {
        expose_accessibility(&app, pid, bundle.as_deref());
    }
    let window_title = app.element("AXFocusedWindow").and_then(|w| w.string("AXTitle"));
    let terminal = bundle.as_deref().is_some_and(crate::platform::focus::is_terminal);
    let field = field_chars.and_then(|n| app.element("AXFocusedUIElement").and_then(|f| surroundings(&f, n, terminal)));
    Focus {
        app: bundle,
        pid: pid.map(|p| p as u32),
        window_title,
        field,
        agent: None,
    }
}

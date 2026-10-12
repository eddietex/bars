//! Linux accessibility (AT-SPI over D-Bus): the focused text field and the
//! text around its cursor, plus the focused app and window title.
//!
//! Best effort. AT-SPI has no "what has focus" call, so a listener follows
//! focus events, and only apps with accessibility on send them: GTK apps
//! do; Qt, Firefox, Chromium and Electron apps only while accessibility is
//! enabled for the desktop, and most terminals expose nothing. Inside
//! Flatpak the accessibility bus is filtered to the app's own objects.

use std::sync::{Arc, OnceLock};
use std::thread;
use std::time::Duration;

use parking_lot::Mutex;
use zbus::blocking::{Connection, MessageIterator};
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Structure, Value};

use crate::platform::focus::Focus;
use crate::text::Surroundings;

/// Each call to an app; a hung app costs at most this per call.
const CALL_TIMEOUT: Duration = Duration::from_millis(250);

const ROLE_DIALOG: u32 = 16;
const ROLE_FRAME: u32 = 23;
const ROLE_PASSWORD_TEXT: u32 = 40;
const ROLE_WINDOW: u32 = 69;
const STATE_FOCUSED: u32 = 12;

const ACCESSIBLE: &str = "org.a11y.atspi.Accessible";
const TEXT: &str = "org.a11y.atspi.Text";

struct A11y {
    conn: Connection,
    /// The object that last gained focus: (bus name, path).
    focused: Arc<Mutex<Option<(String, OwnedObjectPath)>>>,
}

/// Connects to the accessibility bus and starts following focus, once.
fn a11y() -> Option<&'static A11y> {
    static A11Y: OnceLock<Option<A11y>> = OnceLock::new();
    A11Y.get_or_init(|| match connect() {
        Ok(a) => Some(a),
        Err(e) => {
            log::info!("accessibility bus unavailable ({e:#}); smart spacing can't read text fields");
            None
        }
    })
    .as_ref()
}

/// Starts following focus early, so the first dictation already knows.
pub fn warm_up() {
    thread::spawn(|| {
        a11y();
    });
}

fn connect() -> anyhow::Result<A11y> {
    let session = Connection::session()?;
    let reply = session.call_method(Some("org.a11y.Bus"), "/org/a11y/bus", Some("org.a11y.Bus"), "GetAddress", &())?;
    let address: String = reply.body().deserialize()?;
    let conn = zbus::blocking::connection::Builder::address(address.as_str())?
        .method_timeout(CALL_TIMEOUT)
        .build()?;
    // Apps only send the events someone registered for. Newer registries
    // also take properties and an app name.
    const REGISTRY: &str = "org.a11y.atspi.Registry";
    const REGISTRY_PATH: &str = "/org/a11y/atspi/registry";
    let event = "object:state-changed:focused";
    if conn.call_method(Some(REGISTRY), REGISTRY_PATH, Some(REGISTRY), "RegisterEvent", &(event,)).is_err() {
        let body = (event, Vec::<String>::new(), "");
        let _ = conn.call_method(Some(REGISTRY), REGISTRY_PATH, Some(REGISTRY), "RegisterEvent", &body);
    }
    let focused = Arc::new(Mutex::new(None));
    let rule = "type='signal',interface='org.a11y.atspi.Event.Object',member='StateChanged',arg0='focused'";
    let messages = MessageIterator::for_match_rule(rule, &conn, Some(64))?;
    let slot = focused.clone();
    thread::Builder::new().name("bars-a11y".into()).spawn(move || {
        for msg in messages.flatten() {
            let header = msg.header();
            let (Some(sender), Some(path)) = (header.sender(), header.path()) else { continue };
            let body = msg.body();
            let Ok(fields) = body.deserialize::<Structure>() else { continue };
            let gained = matches!(fields.fields().get(1), Some(Value::I32(1)));
            let object = (sender.to_string(), OwnedObjectPath::from(path.to_owned()));
            let mut cur = slot.lock();
            if gained {
                *cur = Some(object);
            } else if cur.as_ref() == Some(&object) {
                *cur = None;
            }
        }
    })?;
    Ok(A11y { conn, focused })
}

impl A11y {
    fn call<B, R>(&self, bus: &str, path: &str, iface: &str, method: &str, body: &B) -> Option<R>
    where
        B: serde::Serialize + zbus::zvariant::DynamicType,
        R: for<'d> zbus::zvariant::DynamicDeserialize<'d>,
    {
        let reply = self.conn.call_method(Some(bus), path, Some(iface), method, body).ok()?;
        reply.body().deserialize::<R>().ok()
    }

    fn property(&self, bus: &str, path: &str, iface: &str, name: &str) -> Option<OwnedValue> {
        self.call(bus, path, "org.freedesktop.DBus.Properties", "Get", &(iface, name))
    }

    fn int(&self, bus: &str, path: &str, iface: &str, name: &str) -> Option<i32> {
        i32::try_from(self.property(bus, path, iface, name)?).ok()
    }

    fn role(&self, bus: &str, path: &str) -> Option<u32> {
        self.call(bus, path, ACCESSIBLE, "GetRole", &())
    }

    fn has_state(&self, bus: &str, path: &str, state: u32) -> bool {
        let words: Vec<u32> = self.call(bus, path, ACCESSIBLE, "GetState", &()).unwrap_or_default();
        words.get((state / 32) as usize).is_some_and(|w| w & (1 << (state % 32)) != 0)
    }

    fn text(&self, bus: &str, path: &str, start: i32, end: i32) -> Option<String> {
        if start >= end {
            return Some(String::new());
        }
        self.call(bus, path, TEXT, "GetText", &(start, end))
    }

    /// The title of the window around an object.
    fn window_title(&self, bus: &str, path: &str) -> Option<String> {
        let mut path = OwnedObjectPath::try_from(path).ok()?;
        for _ in 0..40 {
            if matches!(self.role(bus, &path), Some(ROLE_FRAME | ROLE_WINDOW | ROLE_DIALOG)) {
                return String::try_from(self.property(bus, &path, ACCESSIBLE, "Name")?).ok();
            }
            let parent = self.property(bus, &path, ACCESSIBLE, "Parent")?;
            let Value::Structure(s) = &*parent else { return None };
            let Some(Value::ObjectPath(p)) = s.fields().get(1) else { return None };
            if p.as_str() == "/org/a11y/atspi/null" || p.as_str() == path.as_str() {
                return None;
            }
            path = OwnedObjectPath::from(p.to_owned());
        }
        None
    }

    fn pid_of(&self, bus: &str) -> Option<u32> {
        self.call(
            "org.freedesktop.DBus",
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
            "GetConnectionUnixProcessID",
            &(bus,),
        )
    }

    fn surroundings(&self, bus: &str, path: &str, chars: usize) -> Option<Surroundings> {
        if self.role(bus, path) == Some(ROLE_PASSWORD_TEXT) {
            return None;
        }
        let count = self.int(bus, path, TEXT, "CharacterCount")?;
        // -1: the caret is in a child object (a paragraph of a rich text
        // editor), so the text around it is unknown, not the field's start.
        let caret = self.int(bus, path, TEXT, "CaretOffset")?;
        if caret < 0 {
            return None;
        }
        let caret = caret.min(count);
        let selections: i32 = self.call(bus, path, TEXT, "GetNSelections", &()).unwrap_or(0);
        let (start, end) = if selections > 0 {
            let (a, b): (i32, i32) = self.call(bus, path, TEXT, "GetSelection", &(0i32,))?;
            (a.min(b).clamp(0, count), a.max(b).clamp(0, count))
        } else {
            (caret, caret)
        };
        let n = chars as i32;
        Some(Surroundings {
            before: self.text(bus, path, (start - n).max(0), start)?,
            selected: self.text(bus, path, start, end)?,
            after: self.text(bus, path, end, (end + n).min(count))?,
        })
    }
}

pub fn read_focus(field_chars: Option<usize>) -> Focus {
    let Some(a) = a11y() else { return Focus::default() };
    let Some((bus, path)) = a.focused.lock().clone() else { return Focus::default() };
    // Focus may have moved to an app that sends no events.
    if !a.has_state(&bus, &path, STATE_FOCUSED) {
        return Focus::default();
    }
    let pid = a.pid_of(&bus);
    let app = pid
        .and_then(|p| std::fs::read_to_string(format!("/proc/{p}/comm")).ok())
        .map(|s| s.trim().to_lowercase());
    Focus {
        app,
        pid,
        window_title: a.window_title(&bus, &path),
        field: field_chars.and_then(|n| a.surroundings(&bus, &path, n)),
        agent: None,
    }
}

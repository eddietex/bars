//! `bars --doctor`: a plain-text report of what this session supports.

use std::fmt::Write as _;

use wayland_client::protocol::wl_registry;
use wayland_client::{Connection, Dispatch, QueueHandle};

#[derive(Default)]
struct Globals(Vec<String>);

impl Dispatch<wl_registry::WlRegistry, ()> for Globals {
    fn event(
        state: &mut Self,
        _: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global { interface, .. } = event {
            state.0.push(interface);
        }
    }
}

fn wayland_globals() -> Option<Vec<String>> {
    let conn = Connection::connect_to_env().ok()?;
    let mut queue = conn.new_event_queue();
    conn.display().get_registry(&queue.handle(), ());
    let mut globals = Globals::default();
    queue.roundtrip(&mut globals).ok()?;
    Some(globals.0)
}

fn portal_version(interface: &str) -> Option<u32> {
    let conn = zbus::blocking::Connection::session().ok()?;
    let proxy = zbus::blocking::Proxy::new(
        &conn,
        "org.freedesktop.portal.Desktop",
        "/org/freedesktop/portal/desktop",
        interface.to_string(),
    )
    .ok()?;
    proxy.get_property::<u32>("version").ok()
}

fn mark(ok: bool) -> &'static str {
    if ok { "yes" } else { "no " }
}

pub fn report() -> String {
    let mut out = String::new();
    let env = |k: &str| std::env::var(k).unwrap_or_else(|_| "-".into());
    let _ = writeln!(out, "Bars {} diagnostics", env!("CARGO_PKG_VERSION"));
    let _ = writeln!(
        out,
        "session: {} on {} · sandboxed (Flatpak): {}",
        env("XDG_SESSION_TYPE"),
        env("XDG_CURRENT_DESKTOP"),
        crate::paths::is_flatpak()
    );

    let _ = writeln!(out, "\nWayland protocols");
    match wayland_globals() {
        Some(g) => {
            let has = |name: &str| g.iter().any(|i| i == name);
            let _ = writeln!(out, "  [{}] layer-shell (focus-free overlay)", mark(has("zwlr_layer_shell_v1")));
            let dc = has("ext_data_control_manager_v1") || has("zwlr_data_control_manager_v1");
            let _ = writeln!(out, "  [{}] data-control (clipboard paste + restore)", mark(dc));
        }
        None => {
            let _ = writeln!(out, "  (no Wayland connection)");
        }
    }

    let _ = writeln!(out, "\nInput");
    let hk = super::evdev_hotkeys::can_read_keyboards();
    let ui = super::permissions::can_open_uinput();
    let _ = writeln!(out, "  [{}] read keyboards (direct shortcut backend)", mark(hk));
    let _ = writeln!(out, "  [{}] /dev/uinput (virtual keyboard)", mark(ui));
    if crate::paths::is_flatpak() {
        let _ = writeln!(out, "  [ ? ] udev rule {} (not visible from the sandbox)", super::permissions::RULE_PATH);
    } else {
        let _ = writeln!(out, "  [{}] udev rule {}", mark(super::permissions::rule_installed()), super::permissions::RULE_PATH);
    }
    let _ = writeln!(
        out,
        "  [{}] admin prompt for one-click setup (polkit{})",
        mark(super::permissions::can_elevate()),
        if crate::paths::is_flatpak() { " via host" } else { "" }
    );
    let gs = portal_version("org.freedesktop.portal.GlobalShortcuts");
    let rd = portal_version("org.freedesktop.portal.RemoteDesktop");
    let _ = writeln!(
        out,
        "  [{}] GlobalShortcuts portal{}",
        mark(gs.is_some()),
        gs.map(|v| format!(" v{v}")).unwrap_or_default()
    );
    let _ = writeln!(
        out,
        "  [{}] RemoteDesktop portal{}",
        mark(rd.is_some()),
        rd.map(|v| format!(" v{v}")).unwrap_or_default()
    );

    let _ = writeln!(out, "\nAudio");
    let devices = crate::audio::list_input_devices();
    if devices.is_empty() {
        let _ = writeln!(out, "  (no microphones found)");
    }
    for d in devices {
        let _ = writeln!(out, "  {} {}", if d.is_default { "*" } else { " " }, d.name);
    }

    let _ = writeln!(out, "\nModels ({})", crate::paths::models_dir().display());
    for m in crate::models::CATALOG {
        let _ = writeln!(out, "  [{}] {} ({})", mark(m.is_installed()), m.name, m.id);
    }
    out
}

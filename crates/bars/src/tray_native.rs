//! Tray (Windows notification area) / menu bar (macOS) icon, built on the
//! `tray-icon` crate. Must live on the main thread.

use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

use crate::brand::{TrayState, tray_pixmaps};
use crate::shell::ShellCommand;

const SIZE: i32 = 64;

pub struct TrayHandle {
    icon: TrayIcon,
    toggle: MenuItem,
    setup: MenuItem,
    menu: Menu,
    needs_setup: bool,
}

/// Converts our ARGB32 (network order) pixmaps to RGBA. `ink` recolors
/// the glyph (alpha kept), e.g. dark glyphs for a light Windows taskbar.
fn icon_for(state: TrayState, ink: Option<[u8; 3]>) -> Option<Icon> {
    let (_, argb) = tray_pixmaps(state).iter().find(|(s, _)| *s == SIZE)?;
    let mut rgba = Vec::with_capacity(argb.len());
    for px in argb.chunks_exact(4) {
        let [a, r, g, b] = [px[0], px[1], px[2], px[3]];
        let [r, g, b] = match (state, ink) {
            (TrayState::Idle, Some(ink)) => ink,
            _ => [r, g, b],
        };
        rgba.extend_from_slice(&[r, g, b, a]);
    }
    Icon::from_rgba(rgba, SIZE as u32, SIZE as u32).ok()
}

fn ink() -> Option<[u8; 3]> {
    #[cfg(target_os = "windows")]
    {
        bars_core::platform::win::permissions::taskbar_is_light().then_some([0x1d, 0x1d, 0x22])
    }
    #[cfg(not(target_os = "windows"))]
    {
        None
    }
}

impl TrayHandle {
    pub fn spawn(tx: async_channel::Sender<ShellCommand>) -> Option<TrayHandle> {
        let toggle = MenuItem::with_id("toggle", "Start dictation", true, None);
        let paste = MenuItem::with_id("paste-last", "Paste last transcript", true, None);
        let open = MenuItem::with_id("open", "Open Bars", true, None);
        let setup = MenuItem::with_id("setup", "Finish setup…", true, None);
        let quit = MenuItem::with_id("quit", "Quit Bars", true, None);
        let menu = Menu::new();
        let built = menu.append_items(&[
            &toggle,
            &paste,
            &PredefinedMenuItem::separator(),
            &open,
            &PredefinedMenuItem::separator(),
            &quit,
        ]);
        if let Err(e) = built {
            log::warn!("tray menu: {e}");
        }

        let menu_tx = tx.clone();
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            let cmd = match event.id().0.as_str() {
                "toggle" => ShellCommand::Toggle,
                "paste-last" => ShellCommand::PasteLast,
                "open" => ShellCommand::ShowSettings(None),
                "setup" => ShellCommand::ShowOnboarding,
                "quit" => ShellCommand::Quit,
                _ => return,
            };
            let _ = menu_tx.try_send(cmd);
        }));
        // Windows: left click opens Bars (right click shows the menu).
        let click_tx = tx;
        TrayIconEvent::set_event_handler(Some(move |event: TrayIconEvent| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                if cfg!(target_os = "windows") {
                    let _ = click_tx.try_send(ShellCommand::ShowSettings(None));
                }
            }
        }));

        let mut builder = TrayIconBuilder::new()
            .with_id("bars")
            .with_tooltip("Bars")
            .with_menu(Box::new(menu.clone()));
        if let Some(icon) = icon_for(TrayState::Idle, ink()) {
            // macOS: a template image is tinted by the system to match the menu bar.
            #[cfg(target_os = "macos")]
            {
                builder = builder.with_icon_templated(icon);
            }
            #[cfg(not(target_os = "macos"))]
            {
                builder = builder.with_icon(icon);
            }
        }
        match builder.build() {
            Ok(icon) => Some(TrayHandle {
                icon,
                toggle,
                setup,
                menu,
                needs_setup: false,
            }),
            Err(e) => {
                log::warn!("tray icon unavailable: {e}");
                None
            }
        }
    }

    pub fn set_state(&self, state: TrayState) {
        let icon = icon_for(state, ink());
        #[cfg(target_os = "macos")]
        let _ = if state == TrayState::Idle {
            self.icon.set_icon_templated(icon)
        } else {
            self.icon.set_icon(icon)
        };
        #[cfg(not(target_os = "macos"))]
        let _ = self.icon.set_icon(icon);
        self.toggle.set_text(if state == TrayState::Recording {
            "Stop dictation"
        } else {
            "Start dictation"
        });
        let _ = self.icon.set_tooltip(Some(match state {
            TrayState::Idle => "Bars",
            TrayState::Recording => "Bars: listening…",
            TrayState::Busy => "Bars: transcribing…",
        }));
    }

    pub fn set_needs_setup(&mut self, needs: bool) {
        if needs == self.needs_setup {
            return;
        }
        self.needs_setup = needs;
        if needs {
            let _ = self.menu.prepend_items(&[&self.setup, &PredefinedMenuItem::separator()]);
        } else {
            let _ = self.menu.remove(&self.setup);
            let _ = self.menu.remove_at(0);
        }
    }
}

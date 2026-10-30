//! System tray icon (StatusNotifierItem). Works on KDE, most wlroots bars,
//! and GNOME with the AppIndicator extension.

use crate::brand::{TrayState, tray_pixmaps};
use crate::shell::ShellCommand;

pub struct Tray {
    state: TrayState,
    needs_setup: bool,
    /// The update item's label (`shell::update_label`).
    update: String,
    tx: async_channel::Sender<ShellCommand>,
}

impl ksni::Tray for Tray {
    fn id(&self) -> String {
        bars_core::paths::APP_ID.into()
    }

    fn title(&self) -> String {
        "Bars".into()
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        tray_pixmaps(self.state)
            .iter()
            .map(|(size, data)| ksni::Icon {
                width: *size,
                height: *size,
                data: data.to_vec(),
            })
            .collect()
    }

    fn status(&self) -> ksni::Status {
        match self.state {
            TrayState::Recording => ksni::Status::NeedsAttention,
            _ => ksni::Status::Active,
        }
    }

    fn attention_icon_pixmap(&self) -> Vec<ksni::Icon> {
        self.icon_pixmap()
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        ksni::ToolTip {
            title: "Bars".into(),
            description: match self.state {
                TrayState::Idle => "Ready to dictate".into(),
                TrayState::Recording => "Listening…".into(),
                TrayState::Busy => "Transcribing…".into(),
            },
            ..Default::default()
        }
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        let cmd = if self.needs_setup {
            ShellCommand::ShowOnboarding
        } else {
            ShellCommand::ShowSettings(None)
        };
        let _ = self.tx.send_blocking(cmd);
    }

    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        use ksni::menu::*;
        let recording = self.state == TrayState::Recording;
        let mut items: Vec<ksni::MenuItem<Self>> = Vec::new();
        if self.needs_setup {
            items.push(
                StandardItem {
                    label: "Finish setup…".into(),
                    icon_name: "dialog-warning".into(),
                    activate: Box::new(|t: &mut Self| {
                        let _ = t.tx.send_blocking(ShellCommand::ShowOnboarding);
                    }),
                    ..Default::default()
                }
                .into(),
            );
            items.push(MenuItem::Separator);
        }
        items.extend(vec![
            StandardItem {
                label: if recording { "Stop dictation".into() } else { "Start dictation".into() },
                icon_name: if recording { "media-playback-stop".into() } else { "audio-input-microphone".into() },
                activate: Box::new(|t: &mut Self| {
                    let _ = t.tx.send_blocking(ShellCommand::Toggle);
                }),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Paste last transcript".into(),
                icon_name: "edit-paste".into(),
                activate: Box::new(|t: &mut Self| {
                    let _ = t.tx.send_blocking(ShellCommand::PasteLast);
                }),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: "Open Bars".into(),
                activate: Box::new(|t: &mut Self| {
                    let _ = t.tx.send_blocking(ShellCommand::ShowSettings(None));
                }),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: self.update.clone(),
                activate: Box::new(|t: &mut Self| {
                    let _ = t.tx.send_blocking(ShellCommand::Update);
                }),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: "Quit".into(),
                icon_name: "application-exit".into(),
                activate: Box::new(|t: &mut Self| {
                    let _ = t.tx.send_blocking(ShellCommand::Quit);
                }),
                ..Default::default()
            }
            .into(),
        ]);
        items
    }
}

pub struct TrayHandle(ksni::blocking::Handle<Tray>);

impl TrayHandle {
    pub fn spawn(tx: async_channel::Sender<ShellCommand>) -> Option<TrayHandle> {
        use ksni::blocking::TrayMethods;
        let tray = Tray {
            state: TrayState::Idle,
            needs_setup: false,
            update: crate::shell::update_label(&crate::state::UpdateStatus::Unknown),
            tx,
        };
        // Inside Flatpak the well-known name can't be owned; ksni handles that.
        match tray.disable_dbus_name(bars_core::paths::is_flatpak()).spawn() {
            Ok(h) => Some(TrayHandle(h)),
            Err(e) => {
                log::warn!("tray icon unavailable: {e}");
                None
            }
        }
    }

    pub fn set_state(&self, state: TrayState) {
        self.0.update(|t| t.state = state);
    }

    pub fn set_update(&mut self, label: String) {
        self.0.update(|t| {
            if t.update != label {
                t.update = label;
            }
        });
    }

    pub fn set_needs_setup(&mut self, needs: bool) {
        self.0.update(|t| {
            if t.needs_setup != needs {
                t.needs_setup = needs;
            }
        });
    }
}

//! App-level coordination: owns the settings and overlay windows, keeps the
//! tray icon in sync, and routes tray/D-Bus commands.

use std::time::Duration;

use gpui_kit::*;
use bars_core::config::OverlayMode;
use bars_core::controller::{Command, Phase};

use crate::brand::TrayState;
use crate::onboarding::{self, Step};
use crate::overlay::{self, Overlay};
use crate::settings::{self, Page};
use crate::state::{AppState, StateEvent};
use crate::theme;

#[derive(Clone, Debug)]
pub enum ShellCommand {
    Toggle,
    /// Sent by `bars --cancel`.
    Cancel,
    PasteLast,
    ShowSettings(Option<Page>),
    /// Open the setup assistant (at the first unfinished step).
    ShowOnboarding,
    Quit,
}

/// How the app was started, which decides how to ask for missing setup.
#[derive(Copy, Clone, PartialEq, Eq)]
pub enum Launch {
    /// Started by the user: open a window.
    Interactive,
    /// Started at login: stay quiet, use a notification if needed.
    Background,
}

const SETUP_NOTIFICATION: &str = "bars-setup";

/// Lets any view ask the shell for something (open a window, etc).
pub struct ShellSender(pub async_channel::Sender<ShellCommand>);

impl Global for ShellSender {}

pub fn send(cx: &App, cmd: ShellCommand) {
    if let Some(s) = cx.try_global::<ShellSender>() {
        let _ = s.0.try_send(cmd);
    }
}

pub struct Shell {
    state: Entity<AppState>,
    settings: Option<WindowHandle<gpui_kit::base::Root>>,
    onboarding: Option<WindowHandle<gpui_kit::base::Root>>,
    overlay: Option<WindowHandle<Overlay>>,
    overlay_close: Option<Task<()>>,
    overlay_open: Option<Task<()>>,
    tray: Option<crate::tray::TrayHandle>,
    _subs: Vec<Subscription>,
}

impl Shell {
    pub fn new(state: Entity<AppState>, commands: async_channel::Receiver<ShellCommand>, tray_tx: async_channel::Sender<ShellCommand>, cx: &mut Context<Self>) -> Self {
        let subs = vec![cx.subscribe(&state, |this, _, ev: &StateEvent, cx| this.on_state(ev, cx))];

        // Route tray / D-Bus commands into the app.
        cx.spawn(async move |this, cx| {
            while let Ok(cmd) = commands.recv().await {
                if this.update(cx, |this, cx| this.on_command(cmd, cx)).is_err() {
                    break;
                }
            }
        })
        .detach();

        let tray = crate::tray::TrayHandle::spawn(tray_tx);

        // Notification buttons ("Finish setup") come back here.
        let weak = cx.weak_entity();
        cx.on_system_notification_response(move |response, cx| {
            if response.tag.as_ref() == SETUP_NOTIFICATION {
                let _ = weak.update(cx, |this, cx| this.show_onboarding(cx));
            }
        });

        Shell {
            state,
            settings: None,
            onboarding: None,
            overlay: None,
            overlay_close: None,
            overlay_open: None,
            tray,
            _subs: subs,
        }
    }

    fn on_state(&mut self, ev: &StateEvent, cx: &mut Context<Self>) {
        let overlay_enabled = self.state.read(cx).config.ui.overlay != OverlayMode::Never;
        match ev {
            StateEvent::PhaseChanged(phase) => {
                if let Some(t) = &self.tray {
                    t.set_state(match phase {
                        Phase::Idle => TrayState::Idle,
                        Phase::Recording { .. } => TrayState::Recording,
                        Phase::Transcribing => TrayState::Busy,
                    });
                }
                match phase {
                    Phase::Recording { locked: false } if overlay_enabled => self.show_overlay_soon(cx),
                    Phase::Recording { locked: true } | Phase::Transcribing if overlay_enabled => {
                        self.show_overlay(cx)
                    }
                    // The pill hides itself; keep the surface briefly so the
                    // next dictation shows instantly.
                    Phase::Idle => {
                        self.overlay_open = None;
                        // Linux overlays are click-through when hidden; elsewhere
                        // close soon so the transparent window can't block clicks.
                        let keep = if cfg!(target_os = "linux") { 20_000 } else { 3_200 };
                        self.schedule_overlay_close(Duration::from_millis(keep), cx)
                    }
                    _ => {}
                }
            }
            StateEvent::Notice => {
                let settings_focused = self.settings_window_active(cx);
                if overlay_enabled && !settings_focused {
                    self.show_overlay(cx);
                    if self.state.read(cx).phase == Phase::Idle {
                        self.schedule_overlay_close(Duration::from_millis(3200), cx);
                    }
                }
            }
            StateEvent::ThemeChanged => self.apply_theme(cx),
        }
        let needs = {
            let st = self.state.read(cx);
            st.permissions_known() && !st.setup_complete()
        };
        if let Some(t) = &mut self.tray {
            t.set_needs_setup(needs);
        }
    }

    /// Called once at startup: greet new users, and make sure anyone with
    /// missing setup is guided to it in a way that fits how we launched.
    pub fn startup(&mut self, launch: Launch, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            // Wait for the controller's first permission/model report.
            for _ in 0..50 {
                let known = this.update(cx, |this, cx| this.state.read(cx).permissions_known()).unwrap_or(true);
                if known {
                    break;
                }
                cx.background_executor().timer(Duration::from_millis(60)).await;
            }
            let _ = this.update(cx, |this, cx| {
                let st = this.state.read(cx);
                let first_run = !st.config.onboarded;
                let complete = st.setup_complete();
                let forced = Step::from_env().is_some();
                match (launch, first_run || !complete || forced) {
                    (Launch::Interactive, true) => this.show_onboarding(cx),
                    (Launch::Interactive, false) => {
                        let page = Page::from_env().unwrap_or(Page::Home);
                        this.show_settings(Some(page), cx)
                    }
                    (Launch::Background, true) => this.notify_setup(cx),
                    (Launch::Background, false) => {}
                }
            });
        })
        .detach();
    }

    fn notify_setup(&mut self, cx: &mut Context<Self>) {
        cx.show_system_notification(SystemNotification {
            tag: SETUP_NOTIFICATION.into(),
            title: "Bars needs a moment of your time".into(),
            body: "Finish setup to start dictating with your shortcut.".into(),
            actions: vec![SystemNotificationAction {
                id: "setup".into(),
                label: "Finish setup".into(),
            }],
        });
    }

    pub fn show_onboarding(&mut self, cx: &mut Context<Self>) {
        if let Some(h) = self.onboarding {
            if h.update(cx, |_, window, _| window.activate_window()).is_ok() {
                return;
            }
        }
        let step = self.first_unfinished_step(cx);
        match onboarding::open(self.state.clone(), step, cx) {
            Ok(h) => self.onboarding = Some(h),
            Err(e) => log::error!("could not open setup assistant: {e:#}"),
        }
    }

    fn first_unfinished_step(&self, cx: &App) -> Step {
        if let Some(step) = Step::from_env() {
            return step;
        }
        let st = self.state.read(cx);
        if !st.config.onboarded {
            return Step::Welcome;
        }
        if st.permissions.items.iter().any(|r| !r.ok()) || !st.hotkeys_active {
            return Step::Access;
        }
        if !st.installed.contains(st.config.model.id.as_str()) {
            return Step::Model;
        }
        Step::Welcome
    }

    fn settings_window_active(&self, cx: &mut Context<Self>) -> bool {
        self.settings
            .and_then(|h| h.update(cx, |_, window, _| window.is_window_active()).ok())
            .unwrap_or(false)
    }

    pub fn apply_theme(&mut self, cx: &mut Context<Self>) {
        let pref = self.state.read(cx).config.ui.theme;
        theme::set_pref(pref, cx);
        let dark = theme::wants_dark(pref, cx);
        theme::apply(dark, cx);
        cx.refresh_windows();
    }

    /// Push-to-talk chords are often the start of another shortcut
    /// (e.g. Ctrl+Super+Left); wait a beat so those never flash the pill.
    fn show_overlay_soon(&mut self, cx: &mut Context<Self>) {
        self.overlay_close = None;
        if self.overlay_alive(cx) {
            return;
        }
        self.overlay_open = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_millis(110)).await;
            let _ = this.update(cx, |this, cx| {
                if matches!(this.state.read(cx).phase, Phase::Recording { .. }) {
                    this.show_overlay(cx);
                }
            });
        }));
    }

    fn overlay_alive(&self, cx: &mut Context<Self>) -> bool {
        self.overlay.is_some_and(|h| h.update(cx, |_, _, _| ()).is_ok())
    }

    fn show_overlay(&mut self, cx: &mut Context<Self>) {
        self.overlay_close = None;
        if !self.overlay_alive(cx) {
            self.overlay = overlay::open(self.state.clone(), cx);
        }
    }

    fn schedule_overlay_close(&mut self, after: Duration, cx: &mut Context<Self>) {
        let Some(handle) = self.overlay else { return };
        self.overlay_close = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(after).await;
            let _ = this.update(cx, |this, cx| {
                if this.state.read(cx).phase == Phase::Idle {
                    let _ = handle.update(cx, |_, window, _| window.remove_window());
                    this.overlay = None;
                }
            });
        }));
    }

    pub fn show_settings(&mut self, page: Option<Page>, cx: &mut Context<Self>) {
        if let Some(h) = self.settings {
            let alive = h
                .update(cx, |root, window, cx| {
                    window.activate_window();
                    if let Some(page) = page {
                        if let Ok(view) = root.view().clone().downcast::<settings::SettingsView>() {
                            view.update(cx, |v, cx| v.go(page, cx));
                        }
                    }
                })
                .is_ok();
            if alive {
                return;
            }
        }
        match settings::open(self.state.clone(), page.unwrap_or(Page::Home), cx) {
            Ok(h) => self.settings = Some(h),
            Err(e) => log::error!("could not open settings: {e:#}"),
        }
    }

    fn on_command(&mut self, cmd: ShellCommand, cx: &mut Context<Self>) {
        let state = self.state.read(cx);
        match cmd {
            ShellCommand::Toggle => state.send(Command::Toggle),
            ShellCommand::Cancel => state.send(Command::Cancel),
            ShellCommand::PasteLast => state.send(Command::PasteLast),
            ShellCommand::ShowSettings(page) => self.show_settings(page, cx),
            ShellCommand::ShowOnboarding => self.show_onboarding(cx),
            ShellCommand::Quit => {
                state.send(Command::Shutdown);
                cx.quit();
            }
        }
    }
}

//! UI-side mirror of the controller's state. The controller thread is the
//! source of truth; this entity renders its events and forwards edits back.

use std::collections::{HashMap, HashSet};
use std::time::Instant;

use gpui_kit::*;
use bars_core::audio::{self, InputDevice};
use bars_core::config::Config;
use bars_core::controller::{Command, ControllerHandle, Event, NoticeKind, Phase};
use bars_core::engine::EngineStatus;
use bars_core::history::{Entry, Stats};
use bars_core::hotkey::Chord;
use bars_core::models;
use bars_core::platform::Permissions;

/// Mic level, kept separate so 60 Hz updates only re-render the overlay.
pub struct LevelMeter {
    pub level: f32,
    pub updated: Instant,
}

#[derive(Clone, Debug)]
pub struct Notice {
    pub kind: NoticeKind,
    pub text: SharedString,
    pub at: Instant,
}

/// Emitted for window-level reactions (overlay visibility, etc).
#[derive(Clone, Debug)]
pub enum StateEvent {
    PhaseChanged(Phase),
    Notice,
    ThemeChanged,
}

pub struct AppState {
    pub controller: ControllerHandle,
    pub config: Config,
    pub phase: Phase,
    pub engine: EngineStatus,
    pub downloads: HashMap<String, (u64, u64)>,
    pub download_errors: HashMap<String, String>,
    pub installed: HashSet<&'static str>,
    pub history: Vec<Entry>,
    pub stats: Stats,
    pub permissions: Permissions,
    pub hotkeys_active: bool,
    pub hotkey_error: Option<String>,
    pub hotkey_backend: &'static str,
    pub desktop_shortcut: bool,
    pub desktop_trigger: Option<String>,
    pub capturing: bool,
    pub capture_error: Option<String>,
    pub notice: Option<Notice>,
    pub devices: Vec<InputDevice>,
    pub level: Entity<LevelMeter>,
}

impl EventEmitter<StateEvent> for AppState {}

impl AppState {
    pub fn new(controller: ControllerHandle, config: Config, cx: &mut Context<Self>) -> Self {
        AppState {
            controller,
            config,
            phase: Phase::Idle,
            engine: EngineStatus::Loading(String::new()),
            downloads: HashMap::new(),
            download_errors: HashMap::new(),
            installed: models::installed().into_iter().collect(),
            history: Vec::new(),
            stats: Stats::default(),
            permissions: Permissions::default(),
            hotkeys_active: false,
            hotkey_error: None,
            hotkey_backend: "none",
            desktop_shortcut: false,
            desktop_trigger: None,
            capturing: false,
            capture_error: None,
            notice: None,
            devices: audio::list_input_devices(),
            level: cx.new(|_| LevelMeter {
                level: 0.0,
                updated: Instant::now(),
            }),
        }
    }

    pub fn send(&self, cmd: Command) {
        self.controller.send(cmd);
    }

    /// Edit the config locally and push it to the controller (which saves it).
    pub fn edit_config(&mut self, cx: &mut Context<Self>, f: impl FnOnce(&mut Config)) {
        let theme_before = self.config.ui.theme;
        f(&mut self.config);
        self.send(Command::SetConfig(Box::new(self.config.clone())));
        if self.config.ui.theme != theme_before {
            cx.emit(StateEvent::ThemeChanged);
        }
        cx.notify();
    }

    pub fn refresh_devices(&mut self, cx: &mut Context<Self>) {
        self.devices = audio::list_input_devices();
        cx.notify();
    }

    pub fn start_capture(&mut self, cx: &mut Context<Self>) {
        self.capturing = true;
        self.capture_error = None;
        self.send(Command::CaptureHotkey(true));
        cx.notify();
    }

    pub fn stop_capture(&mut self, cx: &mut Context<Self>) {
        self.capturing = false;
        self.send(Command::CaptureHotkey(false));
        cx.notify();
    }

    fn on_captured(&mut self, chord: Chord, cx: &mut Context<Self>) {
        self.capturing = false;
        match chord.is_valid() {
            Ok(()) => {
                self.capture_error = None;
                self.edit_config(cx, |c| c.hotkey.chord = chord);
            }
            Err(e) => {
                self.capture_error = Some(format!("{e} (pressed {chord})"));
                cx.notify();
            }
        }
    }

    pub fn apply(&mut self, event: Event, cx: &mut Context<Self>) {
        if !matches!(event, Event::Level(_) | Event::History { .. } | Event::Config(_)) {
            log::debug!("ui event: {event:?}");
        }
        match event {
            Event::Level(l) => {
                self.level.update(cx, |m, cx| {
                    m.level = l;
                    m.updated = Instant::now();
                    cx.notify();
                });
                return;
            }
            Event::Phase(p) => {
                self.phase = p;
                cx.emit(StateEvent::PhaseChanged(p));
            }
            Event::Engine(s) => self.engine = s,
            Event::Download { id, done, total } => {
                self.download_errors.remove(&id);
                self.downloads.insert(id, (done, total));
            }
            Event::DownloadFinished { id, error } => {
                self.downloads.remove(&id);
                if let Some(e) = error {
                    if !e.contains("cancelled") {
                        self.download_errors.insert(id, e);
                    }
                }
                self.installed = models::installed().into_iter().collect();
            }
            Event::Captured(chord) => self.on_captured(chord, cx),
            Event::Dictated(entry) => self.history.push(entry),
            Event::History { entries, stats } => {
                // An empty list with stats is a stats-only refresh.
                if !entries.is_empty() || stats.dictations == 0 {
                    self.history = entries;
                }
                self.stats = stats;
            }
            Event::Permissions(p) => self.permissions = p,
            Event::Hotkeys { active, error, backend, desktop_managed, trigger } => {
                self.hotkeys_active = active;
                self.hotkey_error = error;
                self.hotkey_backend = backend;
                self.desktop_shortcut = desktop_managed;
                self.desktop_trigger = trigger;
            }
            Event::Notice { kind, text } => {
                self.notice = Some(Notice {
                    kind,
                    text: text.into(),
                    at: Instant::now(),
                });
                cx.emit(StateEvent::Notice);
            }
            Event::Config(cfg) => self.config = *cfg,
        }
        cx.notify();
    }

    pub fn active_model(&self) -> Option<&'static models::ModelInfo> {
        models::find(&self.config.model.id)
    }

    /// Everything needed before the first dictation works.
    pub fn setup_complete(&self) -> bool {
        self.installed.contains(self.config.model.id.as_str())
            && self.hotkeys_active
            && self.permissions.items.iter().all(|r| r.ok())
    }

    /// Whether the controller has reported permissions yet.
    pub fn permissions_known(&self) -> bool {
        !self.permissions.items.is_empty()
    }

    /// Keycaps for the shortcut as it will actually trigger.
    pub fn shortcut_parts(&self) -> Vec<String> {
        if self.desktop_shortcut {
            match &self.desktop_trigger {
                Some(t) => t.split('+').map(|p| p.trim().to_string()).filter(|p| !p.is_empty()).collect(),
                None => vec!["Not assigned".into()],
            }
        } else {
            self.config.hotkey.chord.parts()
        }
    }
}

/// Starts the task that drains controller events into the state entity.
pub fn pump(state: Entity<AppState>, rx: async_channel::Receiver<Event>, cx: &mut App) {
    cx.spawn(async move |cx| {
        while let Ok(ev) = rx.recv().await {
            // Coalesce bursts of level updates: only the newest matters.
            let mut ev = ev;
            if matches!(ev, Event::Level(_)) {
                while let Ok(next) = rx.try_recv() {
                    if matches!(next, Event::Level(_)) {
                        ev = next;
                    } else {
                        let s = state.clone();
                        cx.update(|cx| s.update(cx, |s, cx| s.apply(ev, cx)));
                        ev = next;
                        break;
                    }
                }
            }
            let s = state.clone();
            cx.update(|cx| s.update(cx, |s, cx| s.apply(ev, cx)));
        }
    })
    .detach();
}

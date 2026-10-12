//! The dictation controller: a single thread that owns all state and turns
//! hotkey gestures, UI commands and engine results into actions. The UI only
//! sends [`Command`]s and renders [`Event`]s, which keeps it thin and makes
//! the whole pipeline usable headless (CLI, tests, other front-ends).

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use chrono::Local;
use crossbeam_channel::{Receiver, Sender, select, unbounded};

use crate::audio::{Audio, Cue};
use crate::config::{Config, InsertionConfig};
use crate::engine::{Engine, EngineEvent, EngineStatus};
use crate::history::{self, History, Stats};
use crate::hotkey::{Activation, Chord, HotkeyEvent, Intent, Mods, Recognizer};
use crate::models;
use crate::paths;
use crate::platform::{self, Hotkeys, Injector, Permissions};
use crate::text::{self, TextProcessor};

/// Requests from the UI (or tray, D-Bus, CLI).
#[derive(Clone, Debug)]
pub enum Command {
    /// Start hands-free dictation, or finish the current one.
    Toggle,
    Finish,
    Cancel,
    SetConfig(Box<Config>),
    /// Start/stop recording the next key chord for the shortcut picker.
    CaptureHotkey(bool),
    /// Open the desktop's shortcut editor (portal backend).
    ConfigureShortcut,
    /// Open the microphone just to show input levels (onboarding).
    MicTest(bool),
    /// Open a system settings page or link.
    OpenUrl(String),
    DownloadModel(String),
    CancelDownload,
    DeleteModel(String),
    RefreshPermissions,
    /// Cheap re-check while the setup assistant waits for the user to grant
    /// something in system settings; acts only if anything changed.
    PollPermissions,
    /// Ask for missing permissions (all, or one where the OS asks per item).
    FixPermissions(Option<platform::requirements::ReqId>),
    /// Re-insert the most recent transcript.
    PasteLast,
    CopyText(String),
    DeleteEntry(u64),
    ClearHistory,
    Shutdown,
}

/// Completions from helper threads back to the controller.
enum Internal {
    DownloadFinished { id: String, error: Option<String> },
    PermissionsFixed(Result<(), String>),
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Phase {
    Idle,
    Recording { locked: bool },
    Transcribing,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NoticeKind {
    Info,
    Warning,
    Error,
}

/// State changes for the UI.
#[derive(Clone, Debug)]
pub enum Event {
    Phase(Phase),
    /// Microphone level 0..1 while recording.
    Level(f32),
    Engine(EngineStatus),
    Download { id: String, done: u64, total: u64 },
    DownloadFinished { id: String, error: Option<String> },
    Captured(Chord),
    Dictated(history::Entry),
    History { entries: Vec<history::Entry>, stats: Stats },
    Permissions(Permissions),
    Hotkeys {
        active: bool,
        error: Option<String>,
        /// "direct", "desktop" or "none".
        backend: &'static str,
        /// The desktop owns the binding; `trigger` describes it.
        desktop_managed: bool,
        trigger: Option<String>,
    },
    Notice { kind: NoticeKind, text: String },
    Config(Box<Config>),
}

pub type EventSender = async_channel::Sender<Event>;

#[derive(Clone)]
pub struct ControllerHandle {
    tx: Sender<Command>,
}

impl ControllerHandle {
    pub fn send(&self, cmd: Command) {
        let _ = self.tx.send(cmd);
    }
}

pub fn spawn(config: Config, ui: EventSender) -> ControllerHandle {
    let (tx, rx) = unbounded();
    let handle = ControllerHandle { tx: tx.clone() };
    thread::Builder::new()
        .name("bars-controller".into())
        .spawn(move || Controller::new(config, ui).run(rx))
        .expect("spawn controller");
    handle
}

struct Pending {
    /// Audio length, for stats.
    audio_secs: f32,
}

enum InsertJob {
    Text { text: String, cfg: InsertionConfig },
    Copy(String),
    /// Dictation started: note which app/window should receive the text.
    RememberTarget,
}

struct Controller {
    cfg: Config,
    ui: EventSender,
    audio: Audio,
    engine: Engine,
    engine_rx: Receiver<EngineEvent>,
    hotkeys: Hotkeys,
    hotkey_rx: Receiver<HotkeyEvent>,
    hotkey_tx: Sender<HotkeyEvent>,
    inserter: Sender<InsertJob>,
    recognizer: Recognizer,
    phase: Phase,
    recording_since: Option<Instant>,
    start_cue_at: Option<Instant>,
    next_request: u64,
    pending: HashMap<u64, Pending>,
    history: History,
    text: TextProcessor,
    download_cancel: Option<Arc<AtomicBool>>,
    capturing: bool,
    mic_testing: bool,
    bound_trigger: Option<String>,
    internal_tx: Sender<Internal>,
    internal_rx: Receiver<Internal>,
    /// What the UI was last told about permissions.
    last_requirements: Vec<platform::requirements::Requirement>,
}

impl Controller {
    fn new(cfg: Config, ui: EventSender) -> Self {
        let level_ui = ui.clone();
        let audio = Audio::spawn(Arc::new(move |level| {
            let _ = level_ui.try_send(Event::Level(level));
        }));
        let (engine_tx, engine_rx) = unbounded();
        let engine = Engine::spawn(engine_tx);
        let (hotkey_tx, hotkey_rx) = unbounded();
        let hotkeys = Hotkeys::start(cfg.hotkey.backend, cfg.hotkey.chord.clone(), hotkey_tx.clone());
        let inserter = spawn_inserter(ui.clone(), &hotkeys);
        let (internal_tx, internal_rx) = unbounded();
        Controller {
            internal_tx,
            internal_rx,
            recognizer: Recognizer::new(cfg.hotkey.activation),
            text: TextProcessor::new(&cfg.text),
            history: History::open(paths::history_file()),
            cfg,
            ui,
            audio,
            engine,
            engine_rx,
            hotkeys,
            hotkey_rx,
            hotkey_tx,
            inserter,
            phase: Phase::Idle,
            recording_since: None,
            start_cue_at: None,
            next_request: 1,
            pending: HashMap::new(),
            download_cancel: None,
            capturing: false,
            mic_testing: false,
            bound_trigger: None,
            last_requirements: Vec::new(),
        }
    }

    fn emit(&self, e: Event) {
        let _ = self.ui.try_send(e);
    }

    fn notice(&self, kind: NoticeKind, text: impl Into<String>) {
        self.emit(Event::Notice { kind, text: text.into() });
    }

    fn set_phase(&mut self, phase: Phase) {
        if self.phase != phase {
            self.phase = phase;
            self.emit(Event::Phase(phase));
        }
    }

    fn run(mut self, rx: Receiver<Command>) {
        self.emit(Event::Config(Box::new(self.cfg.clone())));
        self.emit_permissions();
        self.emit_hotkey_status();
        self.emit_history();
        self.engine.select(&self.cfg.model.id, self.cfg.model.threads, self.cfg.model.unload_after_mins);
        if self.cfg.audio.keep_warm {
            self.audio.set_warm(self.cfg.audio.device.clone(), true);
        }

        loop {
            let deadline = [self.recognizer.deadline(), self.start_cue_at]
                .into_iter()
                .flatten()
                .min();
            let timeout = deadline
                .map(|d| d.saturating_duration_since(Instant::now()))
                .unwrap_or(Duration::from_secs(3600));
            select! {
                recv(rx) -> cmd => match cmd {
                    Ok(Command::Shutdown) | Err(_) => {
                        self.audio.cancel();
                        return;
                    }
                    Ok(cmd) => self.on_command(cmd),
                },
                recv(self.hotkey_rx) -> ev => {
                    if let Ok(ev) = ev {
                        self.on_hotkey(ev);
                    }
                }
                recv(self.engine_rx) -> ev => {
                    if let Ok(ev) = ev {
                        self.on_engine(ev);
                    }
                }
                recv(self.internal_rx) -> ev => {
                    if let Ok(ev) = ev {
                        self.on_internal(ev);
                    }
                }
                default(timeout) => {}
            }
            let now = Instant::now();
            if let Some(intent) = self.recognizer.on_timer(now) {
                self.on_intent(intent, true);
            }
            if self.start_cue_at.is_some_and(|t| now >= t) {
                self.start_cue_at = None;
                if matches!(self.phase, Phase::Recording { .. }) && self.cfg.audio.sounds {
                    self.audio.cue(Cue::Start, self.cfg.audio.sound_volume);
                }
            }
        }
    }

    fn permissions_context(&self) -> platform::requirements::Context {
        platform::requirements::Context {
            hotkey_backend: self.hotkeys.backend_name(),
            prefers_portal: self.cfg.hotkey.backend == crate::config::HotkeyBackendKind::Portal,
        }
    }

    fn emit_permissions(&mut self) {
        let p = platform::check_permissions(&self.permissions_context());
        self.last_requirements = p.items.clone();
        self.emit(Event::Permissions(p));
    }

    fn poll_permissions(&mut self) {
        let p = platform::check_permissions(&self.permissions_context());
        let statuses = |items: &[platform::requirements::Requirement]| {
            items.iter().map(|r| (r.id, r.status)).collect::<Vec<_>>()
        };
        if statuses(&p.items) != statuses(&self.last_requirements) {
            use platform::requirements::ReqId;
            let mic = |items: &[platform::requirements::Requirement]| {
                items.iter().find(|r| r.id == ReqId::Microphone).map(|r| r.status)
            };
            // A stream opened before access was granted stays silent
            // (macOS), so reopen the live microphone test.
            if mic(&p.items) != mic(&self.last_requirements) && self.mic_testing && self.phase == Phase::Idle {
                let device = self.cfg.audio.device.clone();
                self.audio.cancel();
                self.audio.set_warm(device.clone(), false);
                self.audio.start(device, self.cfg.audio.keep_warm);
            }
            // Something was granted (or revoked): restart what depends on it.
            self.refresh_permissions();
        } else if p.items != self.last_requirements {
            self.last_requirements = p.items.clone();
            self.emit(Event::Permissions(p));
        }
    }

    fn emit_hotkey_status(&self) {
        self.emit(Event::Hotkeys {
            active: self.hotkeys.is_active(),
            error: self.hotkeys.error().map(str::to_string),
            backend: self.hotkeys.backend_name(),
            desktop_managed: self.hotkeys.desktop_managed(),
            trigger: self.bound_trigger.clone(),
        });
    }

    fn emit_history(&self) {
        self.emit(Event::History {
            entries: self.history.entries().to_vec(),
            stats: self.history.stats(),
        });
    }

    fn on_hotkey(&mut self, ev: HotkeyEvent) {
        if let HotkeyEvent::Captured(chord) = ev {
            self.capturing = false;
            self.emit(Event::Captured(chord));
            return;
        }
        if let HotkeyEvent::Bound(trigger) = ev {
            self.bound_trigger = (!trigger.is_empty()).then_some(trigger);
            self.emit_hotkey_status();
            return;
        }
        if self.capturing {
            return;
        }
        if let Some(intent) = self.recognizer.on_event(&ev, Instant::now()) {
            self.on_intent(intent, false);
        }
    }

    fn on_intent(&mut self, intent: Intent, from_timer: bool) {
        match intent {
            Intent::Start => self.start_recording(),
            Intent::Lock => {
                if let Phase::Recording { .. } = self.phase {
                    self.start_cue_at = None;
                    self.set_phase(Phase::Recording { locked: true });
                    if self.cfg.audio.sounds {
                        self.audio.cue(Cue::Lock, self.cfg.audio.sound_volume);
                    }
                }
            }
            Intent::Finish => self.finish_recording(),
            Intent::Cancel => {
                let was_tap = from_timer;
                self.cancel_recording(!was_tap);
                if was_tap && self.cfg.hotkey.activation == Activation::HoldOrDoubleTap {
                    self.notice(
                        NoticeKind::Info,
                        format!("Hold {} to talk · double-tap for hands-free", self.cfg.hotkey.chord),
                    );
                }
            }
        }
    }

    fn start_recording(&mut self) {
        if matches!(self.phase, Phase::Recording { .. }) {
            return;
        }
        self.mic_testing = false;
        self.audio.start(self.cfg.audio.device.clone(), self.cfg.audio.keep_warm);
        let _ = self.inserter.send(InsertJob::RememberTarget);
        self.recording_since = Some(Instant::now());
        // Delay the start sound slightly: chords that turn out to be part of
        // another shortcut get cancelled within this window silently.
        self.start_cue_at = Some(Instant::now() + Duration::from_millis(160));
        self.set_phase(Phase::Recording { locked: false });
    }

    fn cancel_recording(&mut self, audible: bool) {
        if !matches!(self.phase, Phase::Recording { .. }) {
            return;
        }
        self.audio.cancel();
        let long_enough = self
            .recording_since
            .is_some_and(|t| t.elapsed() > Duration::from_millis(400));
        if audible && long_enough && self.cfg.audio.sounds && self.start_cue_at.is_none() {
            self.audio.cue(Cue::Cancel, self.cfg.audio.sound_volume);
        }
        self.start_cue_at = None;
        self.recording_since = None;
        self.recognizer.reset();
        self.set_phase(if self.pending.is_empty() { Phase::Idle } else { Phase::Transcribing });
    }

    fn finish_recording(&mut self) {
        if !matches!(self.phase, Phase::Recording { .. }) {
            return;
        }
        self.recognizer.reset();
        self.start_cue_at = None;
        self.recording_since = None;
        let rec = self.audio.stop();
        if self.cfg.audio.sounds {
            self.audio.cue(Cue::Stop, self.cfg.audio.sound_volume);
        }
        let secs = rec.duration().as_secs_f32();
        let loudness = rec.loudest_window_rms();
        log::info!("recorded {secs:.2}s (peak {:.3}, loudest rms {loudness:.4})", rec.peak);
        if secs < 0.25 {
            self.set_phase(if self.pending.is_empty() { Phase::Idle } else { Phase::Transcribing });
            return;
        }
        if loudness < 0.003 {
            self.notice(NoticeKind::Warning, "Didn't hear anything — check your microphone");
            self.set_phase(if self.pending.is_empty() { Phase::Idle } else { Phase::Transcribing });
            return;
        }
        let id = self.next_request;
        self.next_request += 1;
        self.pending.insert(id, Pending { audio_secs: secs });
        self.engine.transcribe(id, rec.samples, self.cfg.model.language.clone());
        self.set_phase(Phase::Transcribing);
    }

    fn on_engine(&mut self, ev: EngineEvent) {
        match ev {
            EngineEvent::Status(s) => self.emit(Event::Engine(s)),
            EngineEvent::Failed { request, error } => {
                self.pending.remove(&request);
                self.notice(NoticeKind::Error, format!("Transcription failed: {error}"));
                self.settle_phase();
            }
            EngineEvent::Done(t) => {
                let Some(p) = self.pending.remove(&t.request) else { return };
                let cleaned = self.text.process(&t.text);
                if cleaned.is_empty() {
                    self.notice(NoticeKind::Info, "No speech recognized");
                    self.settle_phase();
                    return;
                }
                let out = text::finalize(&cleaned, self.cfg.insertion.trailing_space);
                let _ = self.inserter.send(InsertJob::Text {
                    text: out,
                    cfg: self.cfg.insertion.clone(),
                });
                log::info!(
                    "transcribed {:.1}s in {} ms: {:?}",
                    p.audio_secs,
                    t.infer_ms,
                    cleaned
                );
                let entry = history::Entry {
                    id: self.history.next_id(),
                    at: Local::now(),
                    words: text::word_count(&cleaned),
                    raw: (t.text != cleaned).then(|| t.text.clone()),
                    text: cleaned,
                    model: t.model,
                    audio_secs: p.audio_secs,
                    infer_ms: t.infer_ms,
                };
                self.history.append(entry.clone(), self.cfg.ui.save_history);
                self.emit(Event::Dictated(entry));
                self.emit(Event::History {
                    entries: Vec::new(),
                    stats: self.history.stats(),
                });
                self.settle_phase();
            }
        }
    }

    fn settle_phase(&mut self) {
        if matches!(self.phase, Phase::Recording { .. }) {
            return;
        }
        self.set_phase(if self.pending.is_empty() { Phase::Idle } else { Phase::Transcribing });
    }

    fn on_command(&mut self, cmd: Command) {
        match cmd {
            Command::Toggle => match self.phase {
                Phase::Recording { .. } => self.finish_recording(),
                _ => {
                    self.start_recording();
                    self.recognizer.lock_external();
                    self.on_intent(Intent::Lock, false);
                }
            },
            Command::Finish => self.finish_recording(),
            Command::Cancel => self.cancel_recording(true),
            Command::SetConfig(cfg) => self.apply_config(*cfg),
            Command::CaptureHotkey(on) => {
                self.capturing = on;
                self.hotkeys.set_capture(on);
                if on && !self.hotkeys.is_active() {
                    self.notice(NoticeKind::Warning, "Keyboard access is needed to record a shortcut");
                }
            }
            Command::ConfigureShortcut => self.hotkeys.configure(),
            Command::MicTest(on) => {
                if on && self.phase == Phase::Idle {
                    self.audio.start(self.cfg.audio.device.clone(), self.cfg.audio.keep_warm);
                    self.mic_testing = true;
                } else if !on && self.mic_testing {
                    self.mic_testing = false;
                    if self.phase == Phase::Idle {
                        self.audio.cancel();
                    }
                }
            }
            Command::OpenUrl(url) => {
                if let Err(e) = platform::requirements::open_url(&url) {
                    self.notice(NoticeKind::Error, format!("Couldn't open settings: {e:#}"));
                }
            }
            Command::DownloadModel(id) => self.download(id),
            Command::CancelDownload => {
                if let Some(c) = &self.download_cancel {
                    c.store(true, Ordering::Relaxed);
                }
            }
            Command::DeleteModel(id) => {
                if let Some(m) = models::find(&id) {
                    if let Err(e) = m.delete() {
                        self.notice(NoticeKind::Error, format!("Couldn't delete {}: {e}", m.name));
                    }
                    if id == self.cfg.model.id {
                        self.engine.unload();
                        self.emit(Event::Engine(EngineStatus::Missing(id.clone())));
                    }
                    self.emit(Event::DownloadFinished { id, error: None });
                }
            }
            Command::RefreshPermissions => self.refresh_permissions(),
            Command::PollPermissions => self.poll_permissions(),
            Command::FixPermissions(only) => {
                // The polkit prompt can stay open a while; don't block on it.
                let tx = self.internal_tx.clone();
                thread::spawn(move || {
                    let r = platform::fix_permissions(only).map_err(|e| format!("{e:#}"));
                    let _ = tx.send(Internal::PermissionsFixed(r));
                });
            }
            Command::PasteLast => {
                if let Some(last) = self.history.entries().last() {
                    let _ = self.inserter.send(InsertJob::Text {
                        text: text::finalize(&last.text, self.cfg.insertion.trailing_space),
                        cfg: self.cfg.insertion.clone(),
                    });
                }
            }
            Command::CopyText(text) => {
                let _ = self.inserter.send(InsertJob::Copy(text));
            }
            Command::DeleteEntry(id) => {
                self.history.delete(id);
                self.emit_history();
            }
            Command::ClearHistory => {
                self.history.clear();
                self.emit_history();
            }
            Command::Shutdown => {}
        }
    }

    fn refresh_permissions(&mut self) {
        // Prefer direct access once it's granted (it supports more chords).
        let want_direct = self.hotkeys.backend_name() != "direct"
            && matches!(self.cfg.hotkey.backend, crate::config::HotkeyBackendKind::Auto | crate::config::HotkeyBackendKind::Direct);
        if !self.hotkeys.is_active() || want_direct {
            let next = Hotkeys::start(
                self.cfg.hotkey.backend,
                self.cfg.hotkey.chord.clone(),
                self.hotkey_tx.clone(),
            );
            if next.is_active() || !self.hotkeys.is_active() {
                self.hotkeys = next;
                self.bound_trigger = None;
            }
        }
        self.inserter = spawn_inserter(self.ui.clone(), &self.hotkeys);
        self.emit_permissions();
        self.emit_hotkey_status();
    }

    fn apply_config(&mut self, new: Config) {
        let old = std::mem::replace(&mut self.cfg, new);
        if self.cfg.hotkey.backend != old.hotkey.backend {
            self.hotkeys =
                Hotkeys::start(self.cfg.hotkey.backend, self.cfg.hotkey.chord.clone(), self.hotkey_tx.clone());
            self.inserter = spawn_inserter(self.ui.clone(), &self.hotkeys);
            self.bound_trigger = None;
            self.emit_hotkey_status();
            self.emit_permissions();
        } else if self.cfg.hotkey.chord != old.hotkey.chord {
            self.hotkeys.set_chord(self.cfg.hotkey.chord.clone());
        }
        let cfg = &self.cfg;
        if cfg.hotkey.activation != old.hotkey.activation {
            self.recognizer = Recognizer::new(cfg.hotkey.activation);
        }
        if cfg.model != old.model {
            self.engine.select(&cfg.model.id, cfg.model.threads, cfg.model.unload_after_mins);
        }
        if cfg.audio.keep_warm != old.audio.keep_warm || cfg.audio.device != old.audio.device {
            self.audio.set_warm(cfg.audio.device.clone(), cfg.audio.keep_warm);
        }
        if cfg.text != old.text {
            self.text = TextProcessor::new(&cfg.text);
        }
        if let Err(e) = self.cfg.save(&paths::config_file()) {
            self.notice(NoticeKind::Error, format!("Couldn't save settings: {e:#}"));
        }
    }

    fn download(&mut self, id: String) {
        let Some(info) = models::find(&id) else { return };
        if self.download_cancel.is_some() {
            self.notice(NoticeKind::Info, "Another download is in progress");
            return;
        }
        let cancel = Arc::new(AtomicBool::new(false));
        self.download_cancel = Some(cancel.clone());
        let ui = self.ui.clone();
        let done = self.internal_tx.clone();
        thread::Builder::new()
            .name("bars-download".into())
            .spawn(move || {
                let mut last = Instant::now() - Duration::from_secs(1);
                let result = models::download(info, cancel, &mut |done, total| {
                    if last.elapsed() > Duration::from_millis(100) || done == total {
                        last = Instant::now();
                        let _ = ui.try_send(Event::Download {
                            id: info.id.to_string(),
                            done,
                            total,
                        });
                    }
                });
                let _ = done.send(Internal::DownloadFinished {
                    id: info.id.to_string(),
                    error: result.err().map(|e| format!("{e:#}")),
                });
            })
            .expect("spawn download thread");
    }

    fn on_internal(&mut self, ev: Internal) {
        match ev {
            Internal::DownloadFinished { id, error } => {
                self.download_cancel = None;
                if error.is_none() && id == self.cfg.model.id {
                    let m = &self.cfg.model;
                    self.engine.select(&m.id, m.threads, m.unload_after_mins);
                }
                self.emit(Event::DownloadFinished { id, error });
            }
            Internal::PermissionsFixed(Ok(())) => {
                self.refresh_permissions();
                // Elsewhere the OS prompts are still open; polling notices
                // when the user answers.
                #[cfg(target_os = "linux")]
                self.notice(NoticeKind::Info, "Keyboard access granted");
            }
            Internal::PermissionsFixed(Err(e)) => self.notice(NoticeKind::Error, e),
        }
    }
}

fn spawn_inserter(ui: EventSender, hotkeys: &Hotkeys) -> Sender<InsertJob> {
    let (tx, rx) = unbounded::<InsertJob>();
    let held = hotkeys.mods_probe();
    thread::Builder::new()
        .name("bars-insert".into())
        .spawn(move || {
            let mut injector = Injector::new();
            for job in rx {
                match job {
                    InsertJob::Text { text, cfg } => {
                        // Wait for the user to let go of the hotkey modifiers,
                        // otherwise our paste chord would combine with them.
                        match &held {
                            Some(held) => {
                                let deadline = Instant::now() + Duration::from_millis(1500);
                                while held() != Mods::empty() && Instant::now() < deadline {
                                    thread::sleep(Duration::from_millis(15));
                                }
                            }
                            // Backend can't see physical keys (portal): give
                            // fingers a moment to leave the modifiers.
                            None => thread::sleep(Duration::from_millis(150)),
                        }
                        if let Err(e) = injector.insert(&text, &cfg) {
                            log::warn!("insert failed: {e:#}");
                            let _ = ui.try_send(Event::Notice {
                                kind: NoticeKind::Warning,
                                text: format!("{e:#}"),
                            });
                        }
                    }
                    InsertJob::RememberTarget => injector.remember_target(),
                    InsertJob::Copy(text) => {
                        if let Err(e) = injector.copy(&text) {
                            let _ = ui.try_send(Event::Notice {
                                kind: NoticeKind::Error,
                                text: format!("Couldn't copy: {e:#}"),
                            });
                        } else {
                            let _ = ui.try_send(Event::Notice {
                                kind: NoticeKind::Info,
                                text: "Copied".into(),
                            });
                        }
                    }
                }
            }
        })
        .expect("spawn inserter");
    tx
}

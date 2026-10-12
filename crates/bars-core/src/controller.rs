//! The dictation controller: a single thread that owns all state and turns
//! hotkey gestures, UI commands and engine results into actions. The UI only
//! sends [`Command`]s and renders [`Event`]s, which keeps it thin and makes
//! the whole pipeline usable headless (CLI, tests, other front-ends).

use std::collections::{HashMap, HashSet};
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
use crate::platform::focus::{self, Focus};
use crate::platform::{self, Hotkeys, Injector, Inserted, Permissions};
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
    /// A dictation finished (always sent; carries only the text).
    Transcript(String),
    /// A dictation was added to the history (only while history is saved).
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
    /// A paste landed nowhere (no text field had focus); carries the text
    /// so it can be copied.
    PasteFailed(String),
    /// macOS: the app whose Secure Keyboard Entry keeps the shortcut from
    /// working, or None once it's off.
    SecureInput(Option<String>),
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
    Text {
        text: String,
        cfg: InsertionConfig,
        /// Dictionary words, which keep their capital in smart spacing.
        names: Vec<String>,
        /// Read the focused field now, rather than use what was read when
        /// the dictation started (e.g. "Paste last").
        fresh: bool,
    },
    Copy(String),
    /// Dictation started: note which app/window should receive the text,
    /// and read what's around the cursor while the user talks.
    RememberTarget { read_focus: bool, read_field: bool },
}

/// How long insertion waits for the focused field to be read.
const FOCUS_TIMEOUT: Duration = Duration::from_millis(400);
/// How often macOS is asked whether another app holds secure input.
const SECURE_INPUT_POLL: Duration = Duration::from_secs(3);
/// Secure input held this long (not just a password typed in passing) is
/// worth a notice, once per app.
const SECURE_INPUT_NOTICE: Duration = Duration::from_secs(6);

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
    /// The most recent dictation, for "Paste last" (never saved).
    last_text: Option<String>,
    /// What the UI was last told about permissions.
    last_requirements: Vec<platform::requirements::Requirement>,
    /// macOS: while the shortcut backend is down, whether access was granted
    /// at the last check, and when that was.
    hotkey_access: bool,
    hotkey_access_checked: Instant,
    /// macOS: the app holding secure input, since when, and which apps the
    /// user was already told about.
    secure_input: Option<String>,
    secure_input_since: Instant,
    secure_input_checked: Instant,
    secure_input_told: HashSet<String>,
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
            last_text: None,
            hotkey_access: platform::hotkey_access_granted(),
            hotkey_access_checked: Instant::now(),
            secure_input: None,
            secure_input_since: Instant::now(),
            secure_input_checked: Instant::now(),
            secure_input_told: HashSet::new(),
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
            self.sync_hotkeys_recording();
            self.emit(Event::Phase(phase));
        }
    }

    /// Tells the shortcut backend whether (and how) a dictation records.
    fn sync_hotkeys_recording(&self) {
        let recording = matches!(self.phase, Phase::Recording { .. });
        self.hotkeys.set_recording(recording, self.phase == (Phase::Recording { locked: true }));
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
        if self.cfg.insertion.smart_spacing || self.cfg.insertion.split_for_agents {
            platform::prepare_focus_reading();
        }

        loop {
            let deadline = [
                self.recognizer.deadline(),
                self.start_cue_at,
                self.hotkey_access_check_at(),
                self.secure_input_check_at(),
            ]
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
            if self.hotkey_access_check_at().is_some_and(|t| now >= t) {
                self.check_hotkey_access(now);
            }
            if self.secure_input_check_at().is_some_and(|t| now >= t) {
                self.check_secure_input(now);
            }
        }
    }

    /// macOS: when to next ask whether another app holds secure input.
    fn secure_input_check_at(&self) -> Option<Instant> {
        (cfg!(target_os = "macos") && self.hotkeys.is_active())
            .then(|| self.secure_input_checked + SECURE_INPUT_POLL)
    }

    /// Secure Keyboard Entry (Terminal, password managers, password fields)
    /// hides every key from Bars, so the shortcut silently stops working.
    /// Name the app responsible.
    fn check_secure_input(&mut self, now: Instant) {
        self.secure_input_checked = now;
        let owner = platform::secure_input_owner();
        if owner != self.secure_input {
            if let Some(app) = &owner {
                log::info!("{app} turned on secure keyboard entry; the shortcut can't be seen");
            }
            self.secure_input = owner.clone();
            self.secure_input_since = now;
            self.emit(Event::SecureInput(owner));
        } else if let Some(app) = &owner {
            if now.saturating_duration_since(self.secure_input_since) >= SECURE_INPUT_NOTICE
                && self.secure_input_told.insert(app.clone())
            {
                self.notice(NoticeKind::Warning, format!("Shortcut paused: {app} has secure input on"));
            }
        }
    }

    /// macOS: when to next check whether shortcut access has been granted,
    /// while the backend is down (it can't start without it).
    fn hotkey_access_check_at(&self) -> Option<Instant> {
        (cfg!(target_os = "macos") && !self.hotkeys.is_active())
            .then(|| self.hotkey_access_checked + Duration::from_secs(2))
    }

    /// Starts the shortcut backend once access appears, wherever it was
    /// granted (System Settings, not only the setup assistant).
    fn check_hotkey_access(&mut self, now: Instant) {
        self.hotkey_access_checked = now;
        let granted = platform::hotkey_access_granted();
        if granted && !self.hotkey_access {
            log::info!("shortcut access granted; starting the shortcut listener");
            self.refresh_permissions();
        }
        self.hotkey_access = granted;
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
        let ins = &self.cfg.insertion;
        let _ = self.inserter.send(InsertJob::RememberTarget {
            read_focus: ins.smart_spacing || ins.split_for_agents,
            read_field: ins.smart_spacing,
        });
        self.recording_since = Some(Instant::now());
        log::info!("dictation started");
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
                let _ = self.inserter.send(InsertJob::Text {
                    text: cleaned.clone(),
                    cfg: self.cfg.insertion.clone(),
                    names: self.dictionary_words(),
                    fresh: false,
                });
                // What was said stays out of the logs (which can end up in
                // the system journal), unless debugging.
                log::info!(
                    "transcribed {:.1}s in {} ms ({} words)",
                    p.audio_secs,
                    t.infer_ms,
                    text::word_count(&cleaned)
                );
                log::debug!("transcript: {cleaned:?}");
                // Kept in memory only, for "Paste last".
                self.last_text = Some(cleaned.clone());
                self.emit(Event::Transcript(cleaned.clone()));
                // History off: nothing is recorded, on disk or in the
                // history list (which a later rewrite would save).
                if self.cfg.ui.save_history {
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
                    self.history.append(entry.clone());
                    self.emit(Event::Dictated(entry));
                    self.emit(Event::History {
                        entries: Vec::new(),
                        stats: self.history.stats(),
                    });
                }
                self.settle_phase();
            }
        }
    }

    /// What dictionary entries write, which smart spacing never lowercases.
    fn dictionary_words(&self) -> Vec<String> {
        self.cfg.text.dictionary.iter().map(|r| r.to.clone()).collect()
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
                    let what = if cfg!(target_os = "macos") { "Accessibility access" } else { "Keyboard access" };
                    self.notice(NoticeKind::Warning, format!("{what} is needed to record a shortcut"));
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
                let last = self
                    .last_text
                    .clone()
                    .or_else(|| self.history.entries().last().map(|e| e.text.clone()));
                if let Some(last) = last {
                    let _ = self.inserter.send(InsertJob::Text {
                        text: last,
                        cfg: self.cfg.insertion.clone(),
                        names: self.dictionary_words(),
                        fresh: true,
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
                self.sync_hotkeys_recording();
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
            self.sync_hotkeys_recording();
            self.inserter = spawn_inserter(self.ui.clone(), &self.hotkeys);
            self.bound_trigger = None;
            self.emit_hotkey_status();
            self.emit_permissions();
        } else if self.cfg.hotkey.chord != old.hotkey.chord {
            log::info!("shortcut set to {}", self.cfg.hotkey.chord);
            self.hotkeys.set_chord(self.cfg.hotkey.chord.clone());
        }
        let cfg = &self.cfg;
        if cfg.hotkey.activation != old.hotkey.activation {
            self.recognizer = Recognizer::new(cfg.hotkey.activation);
        }
        if cfg.model != old.model {
            self.engine.select(&cfg.model.id, cfg.model.threads, cfg.model.unload_after_mins);
        }
        if cfg.audio.device != old.audio.device && self.mic_testing && self.phase == Phase::Idle {
            // The microphone test (setup assistant) is listening: move it to
            // the newly picked device right away.
            self.audio.cancel();
            self.audio.start(cfg.audio.device.clone(), cfg.audio.keep_warm);
        } else if cfg.audio.keep_warm != old.audio.keep_warm || cfg.audio.device != old.audio.device {
            self.audio.set_warm(cfg.audio.device.clone(), cfg.audio.keep_warm);
        }
        if cfg.text != old.text {
            self.text = TextProcessor::new(&cfg.text);
        }
        let reads_focus = |c: &Config| c.insertion.smart_spacing || c.insertion.split_for_agents;
        if reads_focus(cfg) && !reads_focus(&old) {
            platform::prepare_focus_reading();
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
            // What had focus when the current dictation started.
            let mut started_in: Option<Receiver<Focus>> = None;
            for job in rx {
                match job {
                    InsertJob::Text { text, cfg, names, fresh } => {
                        let wants = cfg.smart_spacing || cfg.split_for_agents;
                        let focus = if fresh {
                            wants.then(|| focus::capture(cfg.smart_spacing, FOCUS_TIMEOUT))
                        } else {
                            started_in.take().and_then(|rx| rx.recv_timeout(FOCUS_TIMEOUT).ok())
                        };
                        insert_text(&mut injector, &ui, held.as_deref(), &text, &cfg, &names, focus.unwrap_or_default());
                    }
                    InsertJob::RememberTarget { read_focus, read_field } => {
                        injector.remember_target();
                        started_in = read_focus.then(|| focus::start_capture(read_field));
                    }
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

/// Fits `text` to the focused field, splits it for a terminal agent, and
/// inserts it.
fn insert_text(
    injector: &mut Injector,
    ui: &EventSender,
    held: Option<&(dyn Fn() -> Mods + Send + Sync)>,
    text: &str,
    cfg: &InsertionConfig,
    names: &[String],
    focus: Focus,
) {
    log::debug!("inserting into {}", focus.summary());
    let text = match focus.field.as_ref().filter(|_| cfg.smart_spacing) {
        Some(field) => text::fit_to_field(text, field, cfg.trailing_space, names),
        None => text::finalize(text, cfg.trailing_space),
    };
    if text.is_empty() {
        return;
    }
    // Typed text never collapses into "[Pasted text]".
    let agent = focus
        .agent
        .filter(|_| cfg.split_for_agents && cfg.method != crate::config::InsertMethod::Type);
    let pieces = match agent {
        Some(agent) => text::split_for_agent(&text, agent),
        None => vec![text.clone()],
    };
    if injector.waits_for_released_keys(cfg) {
        // Wait for the user to let go of the hotkey modifiers, otherwise
        // our paste chord would combine with them.
        match held {
            Some(held) => {
                let deadline = Instant::now() + Duration::from_millis(1500);
                while held() != Mods::empty() && Instant::now() < deadline {
                    thread::sleep(Duration::from_millis(15));
                }
            }
            // Backend can't see physical keys (portal): give fingers a
            // moment to leave the modifiers.
            None => thread::sleep(Duration::from_millis(150)),
        }
    }
    let mut on_unread = || {
        let _ = ui.try_send(Event::PasteFailed(text.trim().to_string()));
    };
    match injector.insert(&pieces, cfg, &mut on_unread) {
        Ok(how) => {
            log::info!(
                "inserted {} characters in {} piece(s) ({:?}, {how:?})",
                text.chars().count(),
                pieces.len(),
                cfg.method
            );
            if how == Inserted::NotRead {
                log::info!("no app read the paste: no text field had focus");
            }
        }
        Err(e) => {
            log::warn!("insert failed: {e:#}");
            let _ = ui.try_send(Event::Notice {
                kind: NoticeKind::Warning,
                text: format!("{e:#}"),
            });
        }
    }
}

//! Global hotkeys by reading keyboards directly (evdev).
//!
//! Works on every compositor (KDE, GNOME, wlroots, X11) and supports
//! modifier-only chords such as Ctrl+Super. Keys are observed, never
//! grabbed, so other apps keep receiving them. Requires read access to
//! `/dev/input/event*` (see `permissions`).

use std::collections::{HashMap, HashSet};
use std::os::fd::{AsFd, AsRawFd, BorrowedFd};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use crossbeam_channel::Sender;
use evdev::{Device, EventSummary, KeyCode};
use nix::poll::{PollFd, PollFlags, PollTimeout, poll};
use parking_lot::Mutex;

use super::keycodes::{key_from_name, modifier_of, name_from_key};
use super::VIRTUAL_KEYBOARD_NAME;
use crate::hotkey::{Chord, ChordTracker, HotkeyEvent, KeyMap, Mods};

#[derive(Default)]
struct Shared {
    chord: Option<Chord>,
    capturing: bool,
    held_mods: Mods,
    stop: bool,
}

/// Handle to the listener thread.
pub struct EvdevHotkeys {
    shared: Arc<Mutex<Shared>>,
}

impl EvdevHotkeys {
    pub fn spawn(chord: Chord, events: Sender<HotkeyEvent>) -> anyhow::Result<Self> {
        // Fail fast (so the caller can fall back) if no keyboard is readable.
        let initial = open_keyboards(&HashMap::new());
        if initial.is_empty() {
            anyhow::bail!("no readable keyboards in /dev/input");
        }
        let shared = Arc::new(Mutex::new(Shared {
            chord: Some(chord),
            ..Default::default()
        }));
        let s = shared.clone();
        thread::Builder::new()
            .name("bars-hotkeys".into())
            .spawn(move || Listener::new(s, events, initial).run())?;
        Ok(EvdevHotkeys { shared })
    }

    pub fn set_chord(&self, chord: Chord) {
        self.shared.lock().chord = Some(chord);
    }

    pub fn set_capture(&self, on: bool) {
        self.shared.lock().capturing = on;
    }

    pub fn held_mods(&self) -> Mods {
        self.shared.lock().held_mods
    }

    pub fn mods_probe(&self) -> Arc<dyn Fn() -> Mods + Send + Sync> {
        let shared = self.shared.clone();
        Arc::new(move || shared.lock().held_mods)
    }
}

impl Drop for EvdevHotkeys {
    fn drop(&mut self) {
        self.shared.lock().stop = true;
    }
}

fn is_keyboard(dev: &Device) -> bool {
    if dev.name() == Some(VIRTUAL_KEYBOARD_NAME) {
        return false;
    }
    dev.supported_keys().is_some_and(|keys| {
        keys.contains(KeyCode::KEY_LEFTCTRL)
            || keys.contains(KeyCode::KEY_LEFTMETA)
            || (keys.contains(KeyCode::KEY_A) && keys.contains(KeyCode::KEY_Z))
    })
}

fn open_keyboards(existing: &HashMap<PathBuf, Device>) -> HashMap<PathBuf, Device> {
    let mut out = HashMap::new();
    let Ok(dir) = std::fs::read_dir("/dev/input") else {
        return out;
    };
    for entry in dir.flatten() {
        let path = entry.path();
        let is_event = path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.starts_with("event"));
        if !is_event || existing.contains_key(&path) {
            continue;
        }
        if let Ok(dev) = Device::open(&path) {
            if is_keyboard(&dev) && dev.set_nonblocking(true).is_ok() {
                log::debug!("hotkeys: watching {} ({})", path.display(), dev.name().unwrap_or("?"));
                out.insert(path, dev);
            }
        }
    }
    out
}

/// Can we read at least one keyboard?
pub fn can_read_keyboards() -> bool {
    !open_keyboards(&HashMap::new()).is_empty()
}

struct EvdevKeys;

impl KeyMap for EvdevKeys {
    fn modifier_of(&self, code: u32) -> Option<Mods> {
        modifier_of(KeyCode::new(code as u16))
    }
    fn name_of(&self, code: u32) -> Option<String> {
        name_from_key(KeyCode::new(code as u16))
    }
    fn code_of(&self, name: &str) -> Option<u32> {
        key_from_name(name).map(|k| k.code() as u32)
    }
    fn is_escape(&self, code: u32) -> bool {
        code == KeyCode::KEY_ESC.code() as u32
    }
}

struct Listener {
    shared: Arc<Mutex<Shared>>,
    events: Sender<HotkeyEvent>,
    devices: HashMap<PathBuf, Device>,
    pressed: HashMap<PathBuf, HashSet<KeyCode>>,
    tracker: ChordTracker,
    last_scan: Instant,
}

impl Listener {
    fn new(shared: Arc<Mutex<Shared>>, events: Sender<HotkeyEvent>, devices: HashMap<PathBuf, Device>) -> Self {
        Listener {
            shared,
            events,
            devices,
            pressed: HashMap::new(),
            tracker: ChordTracker::default(),
            last_scan: Instant::now(),
        }
    }

    fn run(mut self) {
        loop {
            if self.shared.lock().stop {
                return;
            }
            if self.last_scan.elapsed() > Duration::from_secs(3) {
                // Pick up hot-plugged keyboards (and permission changes).
                let new = open_keyboards(&self.devices);
                self.devices.extend(new);
                self.last_scan = Instant::now();
            }

            let paths: Vec<PathBuf> = self.devices.keys().cloned().collect();
            let ready: Vec<PathBuf> = {
                // SAFETY: the fds stay alive for the duration of poll()
                // because `self.devices` is not modified until after.
                let mut fds: Vec<PollFd> = paths
                    .iter()
                    .map(|p| {
                        let fd = self.devices[p].as_fd().as_raw_fd();
                        PollFd::new(unsafe { BorrowedFd::borrow_raw(fd) }, PollFlags::POLLIN)
                    })
                    .collect();
                match poll(&mut fds, PollTimeout::from(500u16)) {
                    Ok(0) => Vec::new(),
                    Ok(_) => fds
                        .iter()
                        .zip(&paths)
                        .filter(|(fd, _)| fd.revents().is_some_and(|r| !r.is_empty()))
                        .map(|(_, p)| p.clone())
                        .collect(),
                    Err(nix::errno::Errno::EINTR) => Vec::new(),
                    Err(e) => {
                        log::error!("hotkeys: poll failed: {e}");
                        thread::sleep(Duration::from_millis(500));
                        Vec::new()
                    }
                }
            };

            for path in ready {
                self.drain(&path);
            }
        }
    }

    fn drain(&mut self, path: &Path) {
        let mut keys: Vec<(KeyCode, i32)> = Vec::new();
        let mut gone = false;
        if let Some(dev) = self.devices.get_mut(path) {
            match dev.fetch_events() {
                Ok(events) => {
                    for ev in events {
                        if let EventSummary::Key(_, code, value) = ev.destructure() {
                            keys.push((code, value));
                        }
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(e) => {
                    log::debug!("hotkeys: {} went away: {e}", path.display());
                    gone = true;
                }
            }
        }
        if gone {
            self.devices.remove(path);
            self.pressed.remove(path);
            self.evaluate(None);
            return;
        }
        for (code, value) in keys {
            let set = self.pressed.entry(path.to_path_buf()).or_default();
            match value {
                1 => {
                    set.insert(code);
                    self.evaluate(Some(code));
                }
                0 => {
                    set.remove(&code);
                    self.evaluate(None);
                }
                _ => {} // autorepeat
            }
        }
    }

    /// `newly_pressed` is the key that just went down, if any.
    fn evaluate(&mut self, newly_pressed: Option<KeyCode>) {
        // Merge keys held on every keyboard.
        let pressed: HashSet<u32> = self.pressed.values().flatten().map(|k| k.code() as u32).collect();
        let (chord, capturing) = {
            let s = self.shared.lock();
            (s.chord.clone(), s.capturing)
        };
        let (events, mods) = self.tracker.update(
            &EvdevKeys,
            &pressed,
            newly_pressed.map(|k| k.code() as u32),
            chord.as_ref(),
            capturing,
        );
        let mut s = self.shared.lock();
        s.held_mods = mods;
        for ev in events {
            if matches!(ev, HotkeyEvent::Captured(_)) {
                s.capturing = false;
            }
            let _ = self.events.send(ev);
        }
    }
}

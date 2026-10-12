//! Platform-independent hotkey model: chords, the events backends emit, and
//! the gesture recognizer that turns raw chord presses into dictation intents
//! (push-to-talk, double-tap hands-free, toggle).

use std::fmt;
use std::str::FromStr;
use std::time::{Duration, Instant};

use serde::{Deserialize, Deserializer, Serialize, Serializer};

bitflags::bitflags! {
    #[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
    pub struct Mods: u8 {
        const CTRL = 1;
        const ALT = 1 << 1;
        const SHIFT = 1 << 2;
        const SUPER = 1 << 3;
    }
}

impl Mods {
    pub fn labels(self) -> Vec<&'static str> {
        let mut out = Vec::new();
        if self.contains(Mods::CTRL) {
            out.push("Ctrl");
        }
        if self.contains(Mods::ALT) {
            out.push("Alt");
        }
        if self.contains(Mods::SHIFT) {
            out.push("Shift");
        }
        if self.contains(Mods::SUPER) {
            out.push(SUPER_LABEL);
        }
        out
    }
}

#[cfg(target_os = "macos")]
const SUPER_LABEL: &str = "Cmd";
#[cfg(target_os = "windows")]
const SUPER_LABEL: &str = "Win";
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
const SUPER_LABEL: &str = "Super";

/// A non-modifier key, named with a platform-neutral canonical name
/// (`"Space"`, `"A"`, `"F9"`, `"Grave"`, ...). Platform backends map these
/// names to native key codes.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Key(pub String);

/// A hotkey: one or more modifiers, optionally plus a single key.
/// A chord without a key (e.g. Ctrl+Super) is a "modifier-only" chord, which is
/// the most comfortable push-to-talk binding because it never types anything.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Chord {
    pub mods: Mods,
    pub key: Option<Key>,
}

impl Default for Chord {
    fn default() -> Self {
        Chord {
            mods: Mods::CTRL | Mods::SUPER,
            key: None,
        }
    }
}

impl Chord {
    pub fn is_valid(&self) -> Result<(), &'static str> {
        if self.mods.is_empty() {
            return Err("Shortcut must include at least one modifier (Ctrl, Alt, Shift or Super)");
        }
        if self.key.is_none() && self.mods.bits().count_ones() < 2 {
            return Err("A modifier-only shortcut needs at least two modifiers");
        }
        Ok(())
    }

    /// Labels for rendering keycaps, e.g. `["Ctrl", "Super", "Space"]`.
    pub fn parts(&self) -> Vec<String> {
        let mut parts: Vec<String> = self.mods.labels().into_iter().map(String::from).collect();
        if let Some(key) = &self.key {
            parts.push(key.0.clone());
        }
        parts
    }
}

impl fmt::Display for Chord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.parts().join("+"))
    }
}

impl FromStr for Chord {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut mods = Mods::empty();
        let mut key = None;
        for part in s.split('+').map(str::trim).filter(|p| !p.is_empty()) {
            match part.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => mods |= Mods::CTRL,
                "alt" | "option" => mods |= Mods::ALT,
                "shift" => mods |= Mods::SHIFT,
                "super" | "meta" | "win" | "cmd" | "command" | "logo" => mods |= Mods::SUPER,
                _ => {
                    if key.is_some() {
                        return Err(format!("more than one non-modifier key in '{s}'"));
                    }
                    key = Some(Key(canonical_key_name(part)));
                }
            }
        }
        Ok(Chord { mods, key })
    }
}

/// Normalizes user-typed key names so `"space"`, `"SPACE"` and `"Space"` match.
pub fn canonical_key_name(name: &str) -> String {
    let lower = name.to_ascii_lowercase();
    if lower.len() == 1 {
        return lower.to_ascii_uppercase();
    }
    if let Some(n) = lower.strip_prefix('f') {
        if n.parse::<u8>().is_ok() {
            return format!("F{n}");
        }
    }
    let mut chars = lower.chars();
    match chars.next() {
        Some(c) => c.to_ascii_uppercase().to_string() + chars.as_str(),
        None => String::new(),
    }
}

impl Serialize for Chord {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for Chord {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

/// Raw events produced by a hotkey backend.
#[derive(Clone, Debug, PartialEq)]
pub enum HotkeyEvent {
    /// The chord became fully pressed.
    Down,
    /// The chord was released.
    Up,
    /// While the chord was held, another key was pressed: the user is most
    /// likely performing an unrelated shortcut (e.g. Ctrl+Super+Left).
    Interrupted,
    /// Escape was pressed (used to cancel a recording).
    Escape,
    /// The backend finished capturing a new chord for the settings UI.
    Captured(Chord),
    /// The desktop bound our shortcut (portal backend); human-readable trigger.
    Bound(String),
}

/// How a platform names its key codes. Implemented by each hotkey backend
/// (evdev codes, Windows virtual keys, macOS key codes).
pub trait KeyMap {
    /// The modifier a key code represents, if any.
    fn modifier_of(&self, code: u32) -> Option<Mods>;
    /// Canonical name ("Space", "A", "F9") for a non-modifier key.
    fn name_of(&self, code: u32) -> Option<String>;
    /// Key code for a canonical name.
    fn code_of(&self, name: &str) -> Option<u32>;
    fn is_escape(&self, code: u32) -> bool;
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum ChordState {
    Inactive,
    Active,
    /// Another key was pressed while the chord was held.
    Interrupted,
}

/// Turns the set of currently pressed keys into chord [`HotkeyEvent`]s, and
/// records new chords for the shortcut picker. Shared by every OS backend.
#[derive(Debug)]
pub struct ChordTracker {
    state: ChordState,
    capture_keys: Vec<u32>,
}

impl Default for ChordTracker {
    fn default() -> Self {
        ChordTracker {
            state: ChordState::Inactive,
            capture_keys: Vec::new(),
        }
    }
}

impl ChordTracker {
    /// Feed the full pressed set after each key change. `newly_pressed` is
    /// the key that just went down (None for releases). Returns events and,
    /// separately, a chord when capture finished.
    pub fn update(
        &mut self,
        map: &dyn KeyMap,
        pressed: &std::collections::HashSet<u32>,
        newly_pressed: Option<u32>,
        chord: Option<&Chord>,
        capturing: bool,
    ) -> (Vec<HotkeyEvent>, Mods) {
        let mods = pressed
            .iter()
            .filter_map(|k| map.modifier_of(*k))
            .fold(Mods::empty(), |a, m| a | m);
        let mut out = Vec::new();

        if capturing {
            self.state = ChordState::Inactive;
            if let Some(k) = newly_pressed {
                if !self.capture_keys.contains(&k) {
                    self.capture_keys.push(k);
                }
            } else if pressed.is_empty() && !self.capture_keys.is_empty() {
                let keys = std::mem::take(&mut self.capture_keys);
                let cmods = keys
                    .iter()
                    .filter_map(|k| map.modifier_of(*k))
                    .fold(Mods::empty(), |a, m| a | m);
                let key = keys
                    .iter()
                    .rev()
                    .find(|k| map.modifier_of(**k).is_none())
                    .and_then(|k| map.name_of(*k))
                    .map(Key);
                out.push(HotkeyEvent::Captured(Chord { mods: cmods, key }));
            }
            return (out, mods);
        }
        self.capture_keys.clear();

        if newly_pressed.is_some_and(|k| map.is_escape(k)) {
            out.push(HotkeyEvent::Escape);
        }
        let Some(chord) = chord else { return (out, mods) };
        let chord_key = match &chord.key {
            Some(k) => match map.code_of(&k.0) {
                Some(code) => Some(code),
                None => return (out, mods), // unmappable key name
            },
            None => None,
        };
        let others: Vec<u32> = pressed.iter().copied().filter(|k| map.modifier_of(*k).is_none()).collect();
        let chord_held = mods.contains(chord.mods) && chord_key.is_none_or(|k| pressed.contains(&k));
        let exact = mods == chord.mods
            && match chord_key {
                None => others.is_empty(),
                Some(k) => others.len() == 1 && others[0] == k,
            };
        self.state = match self.state {
            ChordState::Inactive if exact => {
                out.push(HotkeyEvent::Down);
                ChordState::Active
            }
            ChordState::Inactive => ChordState::Inactive,
            ChordState::Active if exact => ChordState::Active,
            ChordState::Active if chord_held => {
                out.push(HotkeyEvent::Interrupted);
                ChordState::Interrupted
            }
            ChordState::Active | ChordState::Interrupted if !chord_held => {
                out.push(HotkeyEvent::Up);
                ChordState::Inactive
            }
            s => s,
        };
        (out, mods)
    }

    /// Whether the chord is currently held (e.g. to suppress the Start menu).
    pub fn active(&self) -> bool {
        self.state == ChordState::Active
    }
}

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Activation {
    /// Hold to talk; double-tap to lock into hands-free mode.
    #[default]
    HoldOrDoubleTap,
    /// Hold to talk only.
    Hold,
    /// Press once to start, again to stop.
    Toggle,
}

/// What the recognizer wants the controller to do.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Intent {
    /// Start capturing audio (shows the overlay).
    Start,
    /// The recording is now locked in hands-free mode.
    Lock,
    /// Stop and transcribe.
    Finish,
    /// Stop and discard.
    Cancel,
}

/// Holds shorter than this are taps, not push-to-talk.
pub const TAP_MAX: Duration = Duration::from_millis(280);
/// A second press within this window after a tap locks hands-free mode.
pub const DOUBLE_TAP_WINDOW: Duration = Duration::from_millis(400);
/// An interruption later than this is ignored rather than cancelling.
pub const INTERRUPT_GRACE: Duration = Duration::from_millis(600);

#[derive(Copy, Clone, Debug, PartialEq)]
enum Phase {
    Idle,
    /// Chord held, audio capturing.
    Holding { since: Instant },
    /// A short tap happened; waiting to see if a second press follows.
    TapPending { until: Instant },
    /// Second press of a double tap is held; release locks hands-free.
    SecondPress,
    /// Hands-free (or toggle) recording; the next press stops it.
    Locked,
    /// Stop requested on press; finish fires on release so modifiers are
    /// up before text gets inserted.
    Stopping,
}

/// Converts raw [`HotkeyEvent`]s into [`Intent`]s. Pure logic, so it is easy
/// to test and shared by every platform backend.
#[derive(Debug)]
pub struct Recognizer {
    pub activation: Activation,
    phase: Phase,
}

impl Recognizer {
    pub fn new(activation: Activation) -> Self {
        Recognizer {
            activation,
            phase: Phase::Idle,
        }
    }

    pub fn is_idle(&self) -> bool {
        self.phase == Phase::Idle
    }

    /// Forget any in-flight gesture (e.g. after an external stop/cancel).
    pub fn reset(&mut self) {
        self.phase = Phase::Idle;
    }

    /// Mark that the controller started a hands-free recording by other means
    /// (tray menu, D-Bus); the next chord press stops it.
    pub fn lock_external(&mut self) {
        self.phase = Phase::Locked;
    }

    /// When the recognizer is waiting on a timer, the instant it expires.
    pub fn deadline(&self) -> Option<Instant> {
        match self.phase {
            Phase::TapPending { until } => Some(until),
            _ => None,
        }
    }

    pub fn on_timer(&mut self, now: Instant) -> Option<Intent> {
        match self.phase {
            Phase::TapPending { until } if now >= until => {
                self.phase = Phase::Idle;
                Some(Intent::Cancel)
            }
            _ => None,
        }
    }

    pub fn on_event(&mut self, event: &HotkeyEvent, now: Instant) -> Option<Intent> {
        match (self.activation, self.phase, event) {
            (_, _, HotkeyEvent::Captured(_) | HotkeyEvent::Bound(_)) => None,

            // Escape cancels anything in flight.
            (_, Phase::Idle, HotkeyEvent::Escape) => None,
            (_, _, HotkeyEvent::Escape) => {
                self.phase = Phase::Idle;
                Some(Intent::Cancel)
            }

            // ---- Toggle mode ------------------------------------------------
            (Activation::Toggle, Phase::Idle, HotkeyEvent::Down) => {
                self.phase = Phase::Holding { since: now };
                Some(Intent::Start)
            }
            (Activation::Toggle, Phase::Holding { .. }, HotkeyEvent::Up) => {
                self.phase = Phase::Locked;
                Some(Intent::Lock)
            }
            (Activation::Toggle, Phase::Holding { since }, HotkeyEvent::Interrupted) => {
                self.interrupt(since, now)
            }

            // ---- Hold / HoldOrDoubleTap --------------------------------------
            (_, Phase::Idle, HotkeyEvent::Down) => {
                self.phase = Phase::Holding { since: now };
                Some(Intent::Start)
            }
            (_, Phase::Holding { since }, HotkeyEvent::Up) => {
                let held = now.saturating_duration_since(since);
                if held >= TAP_MAX {
                    self.phase = Phase::Idle;
                    Some(Intent::Finish)
                } else if self.activation == Activation::HoldOrDoubleTap {
                    self.phase = Phase::TapPending {
                        until: now + DOUBLE_TAP_WINDOW,
                    };
                    None
                } else {
                    self.phase = Phase::Idle;
                    Some(Intent::Cancel)
                }
            }
            (_, Phase::Holding { since }, HotkeyEvent::Interrupted) => self.interrupt(since, now),
            (_, Phase::TapPending { .. }, HotkeyEvent::Down) => {
                self.phase = Phase::SecondPress;
                None
            }
            (_, Phase::SecondPress, HotkeyEvent::Up) => {
                self.phase = Phase::Locked;
                Some(Intent::Lock)
            }
            (_, Phase::SecondPress, HotkeyEvent::Interrupted) => {
                self.phase = Phase::Idle;
                Some(Intent::Cancel)
            }

            // ---- Locked (hands-free / toggle) ------------------------------
            (_, Phase::Locked, HotkeyEvent::Down) => {
                self.phase = Phase::Stopping;
                None
            }
            (_, Phase::Stopping, HotkeyEvent::Up) => {
                self.phase = Phase::Idle;
                Some(Intent::Finish)
            }
            (_, Phase::Stopping, HotkeyEvent::Interrupted) => {
                // The user hit some other shortcut while hands-free; keep recording.
                self.phase = Phase::Locked;
                None
            }

            _ => None,
        }
    }

    fn interrupt(&mut self, since: Instant, now: Instant) -> Option<Intent> {
        if now.saturating_duration_since(since) < INTERRUPT_GRACE {
            self.phase = Phase::Idle;
            Some(Intent::Cancel)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(base: Instant, ms: u64) -> Instant {
        base + Duration::from_millis(ms)
    }

    #[test]
    fn chord_roundtrip() {
        let c: Chord = "ctrl+super".parse().unwrap();
        assert_eq!(c.mods, Mods::CTRL | Mods::SUPER);
        assert!(c.key.is_none());
        let c: Chord = "Alt+Shift+space".parse().unwrap();
        assert_eq!(c.key, Some(Key("Space".into())));
        let back: Chord = c.to_string().parse().unwrap();
        assert_eq!(back, c);
        assert!("Space".parse::<Chord>().unwrap().is_valid().is_err());
        assert!("Ctrl".parse::<Chord>().unwrap().is_valid().is_err());
        assert!("Ctrl+F9".parse::<Chord>().unwrap().is_valid().is_ok());
    }

    #[test]
    fn push_to_talk() {
        let t = Instant::now();
        let mut r = Recognizer::new(Activation::HoldOrDoubleTap);
        assert_eq!(r.on_event(&HotkeyEvent::Down, t), Some(Intent::Start));
        assert_eq!(r.on_event(&HotkeyEvent::Up, at(t, 1500)), Some(Intent::Finish));
        assert!(r.is_idle());
    }

    #[test]
    fn single_tap_cancels_after_window() {
        let t = Instant::now();
        let mut r = Recognizer::new(Activation::HoldOrDoubleTap);
        r.on_event(&HotkeyEvent::Down, t);
        assert_eq!(r.on_event(&HotkeyEvent::Up, at(t, 100)), None);
        assert!(r.deadline().is_some());
        assert_eq!(r.on_timer(at(t, 600)), Some(Intent::Cancel));
        assert!(r.is_idle());
    }

    #[test]
    fn double_tap_locks_then_tap_finishes() {
        let t = Instant::now();
        let mut r = Recognizer::new(Activation::HoldOrDoubleTap);
        assert_eq!(r.on_event(&HotkeyEvent::Down, t), Some(Intent::Start));
        r.on_event(&HotkeyEvent::Up, at(t, 100));
        r.on_event(&HotkeyEvent::Down, at(t, 250));
        assert_eq!(r.on_event(&HotkeyEvent::Up, at(t, 330)), Some(Intent::Lock));
        assert_eq!(r.on_event(&HotkeyEvent::Down, at(t, 9000)), None);
        assert_eq!(r.on_event(&HotkeyEvent::Up, at(t, 9100)), Some(Intent::Finish));
    }

    #[test]
    fn interrupted_shortcut_cancels() {
        let t = Instant::now();
        let mut r = Recognizer::new(Activation::HoldOrDoubleTap);
        r.on_event(&HotkeyEvent::Down, t);
        assert_eq!(
            r.on_event(&HotkeyEvent::Interrupted, at(t, 120)),
            Some(Intent::Cancel)
        );
        // A late interruption keeps recording.
        r.on_event(&HotkeyEvent::Down, at(t, 2000));
        assert_eq!(r.on_event(&HotkeyEvent::Interrupted, at(t, 4000)), None);
        assert_eq!(r.on_event(&HotkeyEvent::Up, at(t, 5000)), Some(Intent::Finish));
    }

    #[test]
    fn toggle_mode() {
        let t = Instant::now();
        let mut r = Recognizer::new(Activation::Toggle);
        assert_eq!(r.on_event(&HotkeyEvent::Down, t), Some(Intent::Start));
        assert_eq!(r.on_event(&HotkeyEvent::Up, at(t, 50)), Some(Intent::Lock));
        assert_eq!(r.on_event(&HotkeyEvent::Down, at(t, 3000)), None);
        assert_eq!(r.on_event(&HotkeyEvent::Up, at(t, 3050)), Some(Intent::Finish));
    }

    struct TestMap;
    impl KeyMap for TestMap {
        fn modifier_of(&self, code: u32) -> Option<Mods> {
            match code {
                1 => Some(Mods::CTRL),
                2 => Some(Mods::SUPER),
                3 => Some(Mods::SHIFT),
                _ => None,
            }
        }
        fn name_of(&self, code: u32) -> Option<String> {
            match code {
                10 => Some("Space".into()),
                11 => Some("Left".into()),
                _ => None,
            }
        }
        fn code_of(&self, name: &str) -> Option<u32> {
            match name {
                "Space" => Some(10),
                "Left" => Some(11),
                _ => None,
            }
        }
        fn is_escape(&self, code: u32) -> bool {
            code == 99
        }
    }

    fn feed(t: &mut ChordTracker, keys: &mut std::collections::HashSet<u32>, code: u32, down: bool, chord: &Chord, capturing: bool) -> Vec<HotkeyEvent> {
        if down {
            keys.insert(code);
        } else {
            keys.remove(&code);
        }
        t.update(&TestMap, keys, down.then_some(code), Some(chord), capturing).0
    }

    #[test]
    fn tracker_modifier_only_chord() {
        let chord: Chord = "Ctrl+Super".parse().unwrap();
        let (mut t, mut k) = (ChordTracker::default(), Default::default());
        assert!(feed(&mut t, &mut k, 1, true, &chord, false).is_empty());
        assert_eq!(feed(&mut t, &mut k, 2, true, &chord, false), vec![HotkeyEvent::Down]);
        assert_eq!(feed(&mut t, &mut k, 2, false, &chord, false), vec![HotkeyEvent::Up]);
        assert!(feed(&mut t, &mut k, 1, false, &chord, false).is_empty());
    }

    #[test]
    fn tracker_interruption_then_release() {
        let chord: Chord = "Ctrl+Super".parse().unwrap();
        let (mut t, mut k) = (ChordTracker::default(), Default::default());
        feed(&mut t, &mut k, 1, true, &chord, false);
        feed(&mut t, &mut k, 2, true, &chord, false);
        // Ctrl+Super+Left: another shortcut.
        assert_eq!(feed(&mut t, &mut k, 11, true, &chord, false), vec![HotkeyEvent::Interrupted]);
        assert!(feed(&mut t, &mut k, 11, false, &chord, false).is_empty());
        assert_eq!(feed(&mut t, &mut k, 1, false, &chord, false), vec![HotkeyEvent::Up]);
    }

    #[test]
    fn tracker_key_chord_and_capture() {
        let chord: Chord = "Ctrl+Space".parse().unwrap();
        let (mut t, mut k) = (ChordTracker::default(), Default::default());
        feed(&mut t, &mut k, 1, true, &chord, false);
        assert_eq!(feed(&mut t, &mut k, 10, true, &chord, false), vec![HotkeyEvent::Down]);
        assert_eq!(feed(&mut t, &mut k, 10, false, &chord, false), vec![HotkeyEvent::Up]);
        feed(&mut t, &mut k, 1, false, &chord, false);
        // Capture Shift+Super+Space.
        feed(&mut t, &mut k, 3, true, &chord, true);
        feed(&mut t, &mut k, 2, true, &chord, true);
        feed(&mut t, &mut k, 10, true, &chord, true);
        feed(&mut t, &mut k, 10, false, &chord, true);
        feed(&mut t, &mut k, 2, false, &chord, true);
        let ev = feed(&mut t, &mut k, 3, false, &chord, true);
        assert_eq!(ev, vec![HotkeyEvent::Captured("Shift+Super+Space".parse().unwrap())]);
    }

    #[test]
    fn escape_cancels() {
        let t = Instant::now();
        let mut r = Recognizer::new(Activation::HoldOrDoubleTap);
        r.on_event(&HotkeyEvent::Down, t);
        r.on_event(&HotkeyEvent::Up, at(t, 50));
        r.on_event(&HotkeyEvent::Down, at(t, 100));
        r.on_event(&HotkeyEvent::Up, at(t, 150));
        assert_eq!(r.on_event(&HotkeyEvent::Escape, at(t, 2000)), Some(Intent::Cancel));
    }
}

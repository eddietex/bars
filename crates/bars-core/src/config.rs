//! User configuration, persisted as TOML.

use std::path::Path;

use anyhow::Context as _;
use serde::{Deserialize, Serialize};

use crate::hotkey::{Activation, Chord};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub hotkey: HotkeyConfig,
    pub audio: AudioConfig,
    pub model: ModelConfig,
    pub insertion: InsertionConfig,
    pub text: TextConfig,
    pub ui: UiConfig,
    /// Set once the first-run setup has been completed.
    pub onboarded: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            hotkey: HotkeyConfig::default(),
            audio: AudioConfig::default(),
            model: ModelConfig::default(),
            insertion: InsertionConfig::default(),
            text: TextConfig::default(),
            ui: UiConfig::default(),
            onboarded: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct HotkeyConfig {
    pub chord: Chord,
    pub activation: Activation,
    pub backend: HotkeyBackendKind,
}

impl Default for HotkeyConfig {
    fn default() -> Self {
        HotkeyConfig {
            chord: Chord::default(),
            activation: Activation::default(),
            backend: HotkeyBackendKind::Auto,
        }
    }
}

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum HotkeyBackendKind {
    /// Pick the best available backend for this system.
    #[default]
    Auto,
    /// Read keyboards directly (Linux evdev). Supports modifier-only chords
    /// and works on every compositor, but needs input-device access.
    Direct,
    /// XDG GlobalShortcuts portal. Sandbox-friendly; the desktop owns the binding.
    Portal,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AudioConfig {
    /// Input device id; `None` follows the system default.
    pub device: Option<String>,
    /// Keep the microphone open between dictations for zero start latency
    /// (also enables a short pre-roll so the first syllable is never clipped).
    pub keep_warm: bool,
    pub sounds: bool,
    pub sound_volume: f32,
}

impl Default for AudioConfig {
    fn default() -> Self {
        AudioConfig {
            device: None,
            keep_warm: false,
            sounds: true,
            sound_volume: 0.35,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ModelConfig {
    pub id: String,
    /// BCP-47 language hint; `None` = auto-detect where supported.
    pub language: Option<String>,
    /// Inference threads; `None` lets ONNX Runtime decide (physical cores).
    pub threads: Option<usize>,
    /// Free the model from memory after this many idle minutes (0 = never).
    pub unload_after_mins: u32,
}

impl Default for ModelConfig {
    fn default() -> Self {
        ModelConfig {
            id: crate::models::DEFAULT_MODEL.to_string(),
            language: None,
            threads: None,
            unload_after_mins: 0,
        }
    }
}

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum InsertMethod {
    /// Paste through the clipboard when available, otherwise type.
    #[default]
    Auto,
    /// Put text on the clipboard and send the paste shortcut. Instant, and
    /// handles every character, but briefly replaces the clipboard.
    Paste,
    /// Type the text through a virtual keyboard.
    Type,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PasteKeys {
    /// Shift+Insert, with the text placed on both the clipboard and the
    /// primary selection. Works in GUI apps *and* terminals (Linux).
    ShiftInsert,
    /// Ctrl+V (Cmd+V on macOS).
    CtrlV,
    CtrlShiftV,
}

impl Default for PasteKeys {
    fn default() -> Self {
        if cfg!(target_os = "linux") {
            PasteKeys::ShiftInsert
        } else {
            PasteKeys::CtrlV
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct InsertionConfig {
    pub method: InsertMethod,
    pub paste_keys: PasteKeys,
    /// Put the previous clipboard contents back after pasting.
    pub restore_clipboard: bool,
    /// Append a space so consecutive dictations flow together (with smart
    /// spacing: only at the end of a line).
    pub trailing_space: bool,
    /// Read the text around the cursor (accessibility) to fit spacing,
    /// capitalization and the final full stop into the sentence.
    pub smart_spacing: bool,
    /// Paste long text into Claude Code or Codex in pieces, so it shows as
    /// text instead of "[Pasted text]".
    pub split_for_agents: bool,
}

impl Default for InsertionConfig {
    fn default() -> Self {
        InsertionConfig {
            method: InsertMethod::Auto,
            paste_keys: PasteKeys::default(),
            restore_clipboard: true,
            trailing_space: true,
            smart_spacing: true,
            split_for_agents: true,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Replacement {
    /// What the model tends to hear ("chat gee pee tee").
    pub from: String,
    /// What should be written ("ChatGPT").
    pub to: String,
    /// Learned from a correction rather than added by hand.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub learned: bool,
}

/// A correction seen too few times (or too unclear) to learn outright.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Suggestion {
    pub from: String,
    pub to: String,
    pub seen: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Snippet {
    /// Spoken cue ("my email address").
    pub trigger: String,
    /// Inserted text.
    pub expansion: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TextConfig {
    pub remove_fillers: bool,
    /// Turn "new line" / "new paragraph" into line breaks.
    pub voice_commands: bool,
    pub dictionary: Vec<Replacement>,
    pub snippets: Vec<Snippet>,
    /// Learn dictionary words from corrections made after dictating
    /// (Parakeet models only).
    pub learn: bool,
    pub suggestions: Vec<Suggestion>,
    /// Corrections undone or dismissed: never suggested again.
    pub ignored: Vec<Replacement>,
}

impl Default for TextConfig {
    fn default() -> Self {
        TextConfig {
            remove_fillers: true,
            voice_commands: true,
            dictionary: Vec::new(),
            snippets: Vec::new(),
            learn: true,
            suggestions: Vec::new(),
            ignored: Vec::new(),
        }
    }
}

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum OverlayMode {
    /// Show the pill only while recording/transcribing.
    #[default]
    WhileActive,
    /// Keep a tiny idle pill on screen at all times.
    Always,
    Never,
}

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ThemeMode {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct UiConfig {
    pub overlay: OverlayMode,
    pub theme: ThemeMode,
    pub launch_at_login: bool,
    pub save_history: bool,
}

impl Default for UiConfig {
    fn default() -> Self {
        UiConfig {
            overlay: OverlayMode::WhileActive,
            theme: ThemeMode::System,
            launch_at_login: true,
            save_history: true,
        }
    }
}

impl Config {
    pub fn load_or_default(path: &Path) -> Config {
        match std::fs::read_to_string(path) {
            Ok(s) => match toml::from_str(&s) {
                Ok(cfg) => cfg,
                Err(e) => {
                    log::error!("config at {} is invalid ({e}); using defaults", path.display());
                    let _ = std::fs::rename(path, path.with_extension("toml.bad"));
                    Config::default()
                }
            },
            Err(_) => Config::default(),
        }
    }

    /// Atomic save (write to a temp file, then rename).
    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        let text = toml::to_string_pretty(self)?;
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, text).with_context(|| format!("writing {}", tmp.display()))?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let mut cfg = Config::default();
        cfg.text.dictionary.push(Replacement {
            from: "example".into(),
            to: "Example".into(),
            learned: true,
        });
        let s = toml::to_string_pretty(&cfg).unwrap();
        let back: Config = toml::from_str(&s).unwrap();
        assert_eq!(back, cfg);
        assert!(s.contains("chord = \"Ctrl+Super\""));
    }

    #[test]
    fn partial_file_fills_defaults() {
        let cfg: Config = toml::from_str("[hotkey]\nchord = \"Alt+Space\"\n").unwrap();
        assert_eq!(cfg.hotkey.chord.to_string(), "Alt+Space");
        assert_eq!(cfg.audio, AudioConfig::default());
    }
}

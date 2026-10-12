//! Text insertion on Linux: a uinput virtual keyboard (types keys or sends the
//! paste shortcut) plus the Wayland clipboard (data-control protocol).

use std::io::Read;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context as _, anyhow, bail};
use evdev::uinput::VirtualDevice;
use evdev::{AttributeSet, InputEvent, KeyCode};
use wl_clipboard_rs::copy::{self, ClipboardType, MimeType as CopyMime, Source};
use wl_clipboard_rs::paste::{self, ClipboardType as PasteClipboard, MimeType as PasteMime, Seat};

use super::VIRTUAL_KEYBOARD_NAME;
use super::keymap::CharMap;
use super::portal::PortalKeyboard;
use crate::config::{InsertMethod, InsertionConfig, PasteKeys};

pub struct VirtualKeyboard {
    dev: VirtualDevice,
    created: Instant,
}

impl VirtualKeyboard {
    pub fn new() -> anyhow::Result<Self> {
        let mut keys = AttributeSet::<KeyCode>::new();
        // All "keyboard" codes (KEY_ESC..=KEY_MICMUTE).
        for code in 1..=248u16 {
            keys.insert(KeyCode::new(code));
        }
        let dev = VirtualDevice::builder()
            .context("opening /dev/uinput")?
            .name(VIRTUAL_KEYBOARD_NAME)
            .with_keys(&keys)?
            .build()
            .context("creating virtual keyboard")?;
        Ok(VirtualKeyboard {
            dev,
            created: Instant::now(),
        })
    }

    fn settle(&self) {
        // Compositors need a moment to adopt a brand-new input device.
        let age = self.created.elapsed();
        if age < Duration::from_millis(400) {
            thread::sleep(Duration::from_millis(400) - age);
        }
    }

    fn key(&mut self, code: KeyCode, down: bool) -> std::io::Result<()> {
        self.dev
            .emit(&[InputEvent::new(evdev::EventType::KEY.0, code.code(), down as i32)])
    }

    /// Press `mods`, tap `key`, release `mods`.
    pub fn chord(&mut self, mods: &[KeyCode], key: KeyCode) -> std::io::Result<()> {
        self.settle();
        for m in mods {
            self.key(*m, true)?;
        }
        self.key(key, true)?;
        thread::sleep(Duration::from_millis(4));
        self.key(key, false)?;
        for m in mods.iter().rev() {
            self.key(*m, false)?;
        }
        Ok(())
    }

    pub fn type_text(&mut self, text: &str, map: &CharMap) -> anyhow::Result<()> {
        self.settle();
        for c in text.chars() {
            let (mods, key): (Vec<KeyCode>, KeyCode) = match c {
                // Shift+Enter: a line break that won't "send" in chat apps.
                '\n' => (vec![KeyCode::KEY_LEFTSHIFT], KeyCode::KEY_ENTER),
                '\t' => (vec![], KeyCode::KEY_TAB),
                _ => {
                    let Some(s) = map.stroke(c) else {
                        log::debug!("can't type {c:?} on this layout; skipping");
                        continue;
                    };
                    let mut mods = Vec::new();
                    if s.shift {
                        mods.push(KeyCode::KEY_LEFTSHIFT);
                    }
                    if s.altgr {
                        mods.push(KeyCode::KEY_RIGHTALT);
                    }
                    (mods, s.key)
                }
            };
            for m in &mods {
                self.key(*m, true)?;
            }
            self.key(key, true)?;
            self.key(key, false)?;
            for m in mods.iter().rev() {
                self.key(*m, false)?;
            }
            // Pace typing so slow apps (Electron, terminals over SSH) keep up.
            thread::sleep(Duration::from_millis(3));
        }
        Ok(())
    }
}

/// Reads the current clipboard (or primary selection) as text, with a
/// timeout so a hung source app can't stall dictation.
fn read_selection(primary: bool) -> Option<String> {
    let (tx, rx) = crossbeam_channel::bounded(1);
    thread::spawn(move || {
        let kind = if primary {
            PasteClipboard::Primary
        } else {
            PasteClipboard::Regular
        };
        let result = paste::get_contents(kind, Seat::Unspecified, PasteMime::Text)
            .ok()
            .and_then(|(mut pipe, _)| {
                let mut s = String::new();
                pipe.read_to_string(&mut s).ok().map(|_| s)
            });
        let _ = tx.send(result);
    });
    rx.recv_timeout(Duration::from_millis(250)).ok().flatten()
}

fn set_selection(text: &str, kind: ClipboardType) -> anyhow::Result<()> {
    let mut opts = copy::Options::new();
    opts.clipboard(kind);
    opts.copy(Source::Bytes(text.as_bytes().to_vec().into()), CopyMime::Text)
        .map_err(|e| anyhow!("{e}"))
}

/// Whether the compositor exposes a clipboard protocol we can drive
/// (wlr/ext data-control: KDE, wlroots, Hyprland, COSMIC; not GNOME).
pub fn clipboard_available() -> bool {
    match paste::get_contents(PasteClipboard::Regular, Seat::Unspecified, PasteMime::Text) {
        Ok(_) => true,
        Err(paste::Error::ClipboardEmpty | paste::Error::NoMimeType | paste::Error::NoSeats) => true,
        Err(paste::Error::MissingProtocol { .. }) | Err(paste::Error::WaylandConnection(_)) => false,
        Err(e) => {
            log::debug!("clipboard probe: {e}");
            true
        }
    }
}

/// Something that can press keys in the focused app.
trait KeySink {
    fn paste_chord(&mut self, keys: PasteKeys) -> anyhow::Result<()>;
    fn type_text(&mut self, text: &str) -> anyhow::Result<bool>;
}

impl KeySink for VirtualKeyboard {
    fn paste_chord(&mut self, keys: PasteKeys) -> anyhow::Result<()> {
        let (mods, key) = match keys {
            PasteKeys::ShiftInsert => (vec![KeyCode::KEY_LEFTSHIFT], KeyCode::KEY_INSERT),
            PasteKeys::CtrlV => (vec![KeyCode::KEY_LEFTCTRL], KeyCode::KEY_V),
            PasteKeys::CtrlShiftV => (vec![KeyCode::KEY_LEFTCTRL, KeyCode::KEY_LEFTSHIFT], KeyCode::KEY_V),
        };
        Ok(self.chord(&mods, key)?)
    }

    /// Returns false if the text has characters the layout can't type.
    fn type_text(&mut self, text: &str) -> anyhow::Result<bool> {
        let map = CharMap::load();
        if !map.can_type(text) {
            return Ok(false);
        }
        VirtualKeyboard::type_text(self, text, &map)?;
        Ok(true)
    }
}

mod keysym {
    pub const SHIFT_L: u32 = 0xffe1;
    pub const CONTROL_L: u32 = 0xffe3;
    pub const INSERT: u32 = 0xff63;
    pub const RETURN: u32 = 0xff0d;
    pub const TAB: u32 = 0xff09;
    pub const V: u32 = 0x0076;
}

impl KeySink for PortalKeyboard {
    fn paste_chord(&mut self, keys: PasteKeys) -> anyhow::Result<()> {
        let syms: &[u32] = match keys {
            PasteKeys::ShiftInsert => &[keysym::SHIFT_L, keysym::INSERT],
            PasteKeys::CtrlV => &[keysym::CONTROL_L, keysym::V],
            PasteKeys::CtrlShiftV => &[keysym::CONTROL_L, keysym::SHIFT_L, keysym::V],
        };
        let mut seq: Vec<(u32, bool)> = syms.iter().map(|s| (*s, true)).collect();
        seq.extend(syms.iter().rev().map(|s| (*s, false)));
        self.send(seq)
    }

    fn type_text(&mut self, text: &str) -> anyhow::Result<bool> {
        let mut seq = Vec::new();
        for c in text.chars() {
            match c {
                '\n' => seq.extend([
                    (keysym::SHIFT_L, true),
                    (keysym::RETURN, true),
                    (keysym::RETURN, false),
                    (keysym::SHIFT_L, false),
                ]),
                '\t' => seq.extend([(keysym::TAB, true), (keysym::TAB, false)]),
                _ => {
                    let sym = xkbcommon::xkb::utf32_to_keysym(c as u32).raw();
                    seq.extend([(sym, true), (sym, false)]);
                }
            }
        }
        self.send(seq)?;
        Ok(true)
    }
}

pub struct LinuxInjector {
    keyboard: Option<VirtualKeyboard>,
    portal: Option<PortalKeyboard>,
    clipboard: bool,
}

impl LinuxInjector {
    pub fn new() -> Self {
        let keyboard = match VirtualKeyboard::new() {
            Ok(k) => Some(k),
            Err(e) => {
                log::info!("virtual keyboard unavailable ({e:#}); will use the RemoteDesktop portal");
                None
            }
        };
        LinuxInjector {
            keyboard,
            portal: None,
            clipboard: clipboard_available(),
        }
    }

    pub fn has_keyboard(&self) -> bool {
        self.keyboard.is_some()
    }

    pub fn has_clipboard(&self) -> bool {
        self.clipboard
    }

    pub fn copy_only(&self, text: &str) -> anyhow::Result<()> {
        set_selection(text, ClipboardType::Regular)
    }

    pub fn insert(&mut self, text: &str, cfg: &InsertionConfig) -> anyhow::Result<()> {
        if text.is_empty() {
            return Ok(());
        }
        let clipboard = self.clipboard;
        let sink: &mut dyn KeySink = match self.keyboard.as_mut() {
            Some(kb) => kb,
            None => self.portal.get_or_insert_with(PortalKeyboard::new),
        };

        let method = match cfg.method {
            InsertMethod::Auto if clipboard => InsertMethod::Paste,
            InsertMethod::Auto => InsertMethod::Type,
            InsertMethod::Paste if !clipboard => InsertMethod::Type,
            m => m,
        };

        match method {
            InsertMethod::Type => {
                if sink.type_text(text)? {
                    Ok(())
                } else if clipboard {
                    // Emoji / characters missing from the layout: paste instead.
                    paste_text(sink, text, cfg)
                } else {
                    bail!("some characters can't be typed on this keyboard layout")
                }
            }
            _ => paste_text(sink, text, cfg),
        }
    }
}

fn paste_text(sink: &mut dyn KeySink, text: &str, cfg: &InsertionConfig) -> anyhow::Result<()> {
    let use_primary = cfg.paste_keys == PasteKeys::ShiftInsert;
    let saved = cfg.restore_clipboard.then(|| {
        (
            read_selection(false),
            if use_primary { read_selection(true) } else { None },
        )
    });

    let kind = if use_primary {
        ClipboardType::Both
    } else {
        ClipboardType::Regular
    };
    set_selection(text, kind)?;
    // Let the compositor announce the new selection to the focused client.
    thread::sleep(Duration::from_millis(40));
    sink.paste_chord(cfg.paste_keys)?;

    if let Some((clip, primary)) = saved {
        let ours = text.to_string();
        // Restore after the target app has had time to read the paste.
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(450));
            // If the user copied something new meanwhile, leave it alone.
            if read_selection(false).as_deref() != Some(ours.as_str()) {
                return;
            }
            if let Some(prev) = clip.filter(|s| !s.is_empty()) {
                let _ = set_selection(&prev, ClipboardType::Regular);
            }
            if let Some(prev) = primary.filter(|s| !s.is_empty()) {
                let _ = set_selection(&prev, ClipboardType::Primary);
            }
        });
    }
    Ok(())
}

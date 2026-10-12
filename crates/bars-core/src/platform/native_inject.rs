//! Text insertion on Windows and macOS: the clipboard plus the platform's
//! paste shortcut (Ctrl+V / Cmd+V), or typing through synthetic key events.

use std::thread;
use std::time::Duration;

use anyhow::anyhow;
use arboard::Clipboard;
use enigo::{Direction, Enigo, Key, Keyboard, Settings};

use crate::config::{InsertMethod, InsertionConfig};

/// Runs keyboard simulation where the platform allows it: macOS aborts the
/// app when keyboard layouts (needed to find the key for "v", or to type)
/// are looked up off the main thread; elsewhere any thread works.
fn on_input_thread<R>(f: impl FnOnce() -> R) -> R {
    #[cfg(target_os = "macos")]
    {
        super::mac::main_thread::run(f)
    }
    #[cfg(not(target_os = "macos"))]
    {
        f()
    }
}

#[cfg(target_os = "macos")]
const PASTE_MODIFIER: Key = Key::Meta;
#[cfg(not(target_os = "macos"))]
const PASTE_MODIFIER: Key = Key::Control;

pub struct NativeInjector {
    enigo: Option<Enigo>,
    error: Option<String>,
    /// The window that had focus when dictation started (Windows).
    #[cfg(target_os = "windows")]
    target: Option<isize>,
}

impl NativeInjector {
    pub fn new() -> Self {
        let settings = Settings {
            // The setup assistant asks for permissions; don't pop another prompt.
            open_prompt_to_get_permissions: false,
            ..Settings::default()
        };
        let (enigo, error) = match on_input_thread(|| Enigo::new(&settings)) {
            Ok(e) => (Some(e), None),
            Err(e) => {
                log::warn!("keyboard simulation unavailable: {e}");
                (None, Some(e.to_string()))
            }
        };
        NativeInjector {
            enigo,
            error,
            #[cfg(target_os = "windows")]
            target: None,
        }
    }

    pub fn has_keyboard(&self) -> bool {
        self.enigo.is_some()
    }

    /// Remember where the user is typing, so a click on the overlay can't
    /// steal the paste (Windows activates windows on click).
    pub fn remember_target(&mut self) {
        #[cfg(target_os = "windows")]
        {
            use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;
            self.target = Some(unsafe { GetForegroundWindow() }.0 as isize);
        }
    }

    #[cfg(target_os = "windows")]
    fn restore_target(&self) {
        use windows::Win32::Foundation::HWND;
        use windows::Win32::System::Threading::GetCurrentProcessId;
        use windows::Win32::UI::WindowsAndMessaging::{
            GetForegroundWindow, GetWindowThreadProcessId, SetForegroundWindow,
        };
        let Some(target) = self.target else { return };
        unsafe {
            let fg = GetForegroundWindow();
            let mut pid = 0u32;
            GetWindowThreadProcessId(fg, Some(&mut pid));
            if pid == GetCurrentProcessId() && fg.0 as isize != target {
                let _ = SetForegroundWindow(HWND(target as *mut core::ffi::c_void));
                thread::sleep(Duration::from_millis(80));
            }
        }
    }

    pub fn copy_only(&self, text: &str) -> anyhow::Result<()> {
        Clipboard::new()?.set_text(text.to_string())?;
        Ok(())
    }

    pub fn insert(&mut self, text: &str, cfg: &InsertionConfig) -> anyhow::Result<()> {
        if text.is_empty() {
            return Ok(());
        }
        #[cfg(target_os = "windows")]
        self.restore_target();
        let Some(enigo) = self.enigo.as_mut() else {
            // Still useful: leave the text on the clipboard.
            Clipboard::new()?.set_text(text.to_string())?;
            anyhow::bail!(
                "copied to clipboard ({})",
                self.error.as_deref().unwrap_or("keyboard simulation unavailable")
            );
        };
        match cfg.method {
            InsertMethod::Type => on_input_thread(|| type_text(enigo, text)),
            InsertMethod::Auto | InsertMethod::Paste => paste_text(enigo, text, cfg),
        }
    }
}

impl Default for NativeInjector {
    fn default() -> Self {
        Self::new()
    }
}

fn key_err(e: enigo::InputError) -> anyhow::Error {
    anyhow!("sending keys failed: {e}")
}

fn paste_text(enigo: &mut Enigo, text: &str, cfg: &InsertionConfig) -> anyhow::Result<()> {
    let mut clipboard = Clipboard::new()?;
    let saved = if cfg.restore_clipboard { clipboard.get_text().ok() } else { None };
    clipboard.set_text(text.to_string())?;
    thread::sleep(Duration::from_millis(30));

    on_input_thread(|| {
        enigo.key(PASTE_MODIFIER, Direction::Press).map_err(key_err)?;
        let tapped = enigo.key(Key::Unicode('v'), Direction::Click).map_err(key_err);
        enigo.key(PASTE_MODIFIER, Direction::Release).map_err(key_err)?;
        tapped
    })?;

    if let Some(previous) = saved {
        let ours = text.to_string();
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(450));
            let Ok(mut cb) = Clipboard::new() else { return };
            // Leave it alone if the user copied something new meanwhile.
            if cb.get_text().ok().as_deref() == Some(ours.as_str()) {
                let _ = cb.set_text(previous);
            }
        });
    }
    Ok(())
}

fn type_text(enigo: &mut Enigo, text: &str) -> anyhow::Result<()> {
    for (i, line) in text.split('\n').enumerate() {
        if i > 0 {
            // Shift+Enter: a line break that won't "send" in chat apps.
            enigo.key(Key::Shift, Direction::Press).map_err(key_err)?;
            enigo.key(Key::Return, Direction::Click).map_err(key_err)?;
            enigo.key(Key::Shift, Direction::Release).map_err(key_err)?;
        }
        if !line.is_empty() {
            enigo.text(line).map_err(key_err)?;
        }
    }
    Ok(())
}

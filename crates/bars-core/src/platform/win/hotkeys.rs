//! Global shortcut via a low-level keyboard hook (WH_KEYBOARD_LL). Keys are
//! observed, not swallowed, and no special permission is needed.

use std::cell::RefCell;
use std::collections::HashSet;
use std::sync::Arc;
use std::thread;

use crossbeam_channel::Sender;
use parking_lot::Mutex;
use windows::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    INPUT, INPUT_0, INPUT_KEYBOARD, KEYBD_EVENT_FLAGS, KEYBDINPUT, KEYEVENTF_KEYUP, SendInput,
    VIRTUAL_KEY,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DispatchMessageW, GetMessageW, HHOOK, KBDLLHOOKSTRUCT, LLKHF_INJECTED, MSG,
    SetWindowsHookExW, TranslateMessage, WH_KEYBOARD_LL, WM_KEYDOWN, WM_KEYUP, WM_SYSKEYDOWN,
    WM_SYSKEYUP,
};

use super::keycodes::{VK_UNASSIGNED, WinKeys};
use crate::hotkey::{Chord, ChordTracker, HotkeyEvent, Mods};

#[derive(Default)]
struct Shared {
    chord: Option<Chord>,
    capturing: bool,
    held_mods: Mods,
}

struct HookState {
    shared: Arc<Mutex<Shared>>,
    events: Sender<HotkeyEvent>,
    pressed: HashSet<u32>,
    tracker: ChordTracker,
}

thread_local! {
    // The hook procedure runs on the thread that installed it.
    static STATE: RefCell<Option<HookState>> = const { RefCell::new(None) };
}

pub struct WinHotkeys {
    shared: Arc<Mutex<Shared>>,
}

impl WinHotkeys {
    pub fn spawn(chord: Chord, events: Sender<HotkeyEvent>) -> anyhow::Result<Self> {
        let shared = Arc::new(Mutex::new(Shared {
            chord: Some(chord),
            ..Default::default()
        }));
        let (ready_tx, ready_rx) = crossbeam_channel::bounded::<Result<(), String>>(1);
        let s = shared.clone();
        thread::Builder::new()
            .name("bars-hotkeys".into())
            .spawn(move || {
                STATE.with(|st| {
                    *st.borrow_mut() = Some(HookState {
                        shared: s,
                        events,
                        pressed: HashSet::new(),
                        tracker: ChordTracker::default(),
                    })
                });
                let hook: HHOOK = match unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook_proc), None, 0) } {
                    Ok(h) => h,
                    Err(e) => {
                        let _ = ready_tx.send(Err(format!("keyboard hook: {e}")));
                        return;
                    }
                };
                let _ = ready_tx.send(Ok(()));
                let _ = hook;
                // Low-level hooks are delivered through this thread's queue.
                let mut msg = MSG::default();
                unsafe {
                    while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                        let _ = TranslateMessage(&msg);
                        DispatchMessageW(&msg);
                    }
                }
            })?;
        ready_rx
            .recv()
            .map_err(|_| anyhow::anyhow!("hotkey thread exited"))?
            .map_err(|e| anyhow::anyhow!(e))?;
        Ok(WinHotkeys { shared })
    }

    pub fn set_chord(&self, chord: Chord) {
        self.shared.lock().chord = Some(chord);
    }

    pub fn set_capture(&self, on: bool) {
        self.shared.lock().capturing = on;
    }

    pub fn mods_probe(&self) -> Arc<dyn Fn() -> Mods + Send + Sync> {
        let shared = self.shared.clone();
        Arc::new(move || shared.lock().held_mods)
    }
}

unsafe extern "system" fn hook_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 {
        let kb = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
        // Ignore our own synthetic input (paste chords, typed text).
        if (kb.flags.0 & LLKHF_INJECTED.0) == 0 {
            let msg = wparam.0 as u32;
            let down = msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN;
            let up = msg == WM_KEYUP || msg == WM_SYSKEYUP;
            if down || up {
                on_key(kb.vkCode, down);
            }
        }
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

fn on_key(vk: u32, down: bool) {
    STATE.with(|st| {
        let mut guard = st.borrow_mut();
        let Some(state) = guard.as_mut() else { return };
        // Autorepeat sends repeated key-downs; only the first counts.
        let newly = if down {
            state.pressed.insert(vk).then_some(vk)
        } else {
            state.pressed.remove(&vk);
            None
        };
        if down && newly.is_none() {
            return;
        }
        let (chord, capturing) = {
            let s = state.shared.lock();
            (s.chord.clone(), s.capturing)
        };
        let (events, mods) = state
            .tracker
            .update(&WinKeys, &state.pressed, newly, chord.as_ref(), capturing);
        {
            let mut s = state.shared.lock();
            s.held_mods = mods;
            if events.iter().any(|e| matches!(e, HotkeyEvent::Captured(_))) {
                s.capturing = false;
            }
        }
        for ev in events {
            if ev == HotkeyEvent::Down && chord.as_ref().is_some_and(|c| c.mods.contains(Mods::SUPER)) {
                suppress_start_menu();
            }
            let _ = state.events.send(ev);
        }
    });
}

/// Releasing the Windows key after using it alone opens the Start menu. A
/// tap of an unassigned key in between tells the shell it was a combination.
fn suppress_start_menu() {
    // Not from inside the hook callback: do it off-thread.
    thread::spawn(|| {
        let key = |flags: KEYBD_EVENT_FLAGS| INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: VIRTUAL_KEY(VK_UNASSIGNED),
                    wScan: 0,
                    dwFlags: flags,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        };
        let inputs = [key(KEYBD_EVENT_FLAGS(0)), key(KEYEVENTF_KEYUP)];
        unsafe {
            SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
        }
    });
}

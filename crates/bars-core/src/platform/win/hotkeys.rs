//! Global shortcut via a low-level keyboard hook (WH_KEYBOARD_LL). Keys are
//! observed, and a few swallowed: Escape while recording (it cancels the
//! dictation instead of also reaching the app), the key that finishes the
//! shortcut, and keys pressed while a new shortcut is being recorded.
//! Modifiers always pass, so none can get stuck. No permission is needed.

use std::cell::RefCell;
use std::sync::Arc;
use std::thread;

use crossbeam_channel::Sender;
use parking_lot::Mutex;
use windows::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBD_EVENT_FLAGS, KEYBDINPUT, KEYEVENTF_KEYUP, SendInput,
    VIRTUAL_KEY,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DispatchMessageW, GetMessageW, HHOOK, KBDLLHOOKSTRUCT, LLKHF_INJECTED, MSG, SetTimer,
    SetWindowsHookExW, TranslateMessage, WH_KEYBOARD_LL, WM_KEYDOWN, WM_KEYUP, WM_SYSKEYDOWN, WM_SYSKEYUP, WM_TIMER,
};

use super::keycodes::{VK_UNASSIGNED, WinKeys};
use crate::hotkey::{
    Chord, ChordTracker, FilterInput, HeldKeys, HotkeyEvent, KeyChange, KeyFilter, Mods, STALE_KEY_AGE, STALE_KEY_SWEEP,
};

#[derive(Default)]
struct Shared {
    chord: Option<Chord>,
    capturing: bool,
    recording: bool,
}

struct HookState {
    shared: Arc<Mutex<Shared>>,
    events: Sender<HotkeyEvent>,
    held: HeldKeys,
    tracker: ChordTracker,
    filter: KeyFilter,
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
                        held: HeldKeys::default(),
                        tracker: ChordTracker::default(),
                        filter: KeyFilter::default(),
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
                // Checks for keys whose release was missed.
                unsafe { SetTimer(None, 0, STALE_KEY_SWEEP.as_millis() as u32, None) };
                // Low-level hooks are delivered through this thread's queue.
                let mut msg = MSG::default();
                unsafe {
                    while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                        if msg.message == WM_TIMER {
                            sweep();
                            continue;
                        }
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

    pub fn set_recording(&self, on: bool) {
        self.shared.lock().recording = on;
    }

    /// Modifiers held right now, as Windows reports them.
    pub fn mods_probe(&self) -> Arc<dyn Fn() -> Mods + Send + Sync> {
        Arc::new(|| {
            [
                (0x10, Mods::SHIFT),
                (0x11, Mods::CTRL),
                (0x12, Mods::ALT),
                (0x5B, Mods::SUPER),
                (0x5C, Mods::SUPER),
            ]
            .into_iter()
            .filter(|(vk, _)| key_is_down(*vk))
            .fold(Mods::empty(), |a, (_, m)| a | m)
        })
    }
}

fn key_is_down(vk: u32) -> bool {
    (unsafe { GetAsyncKeyState(vk as i32) } as u16 & 0x8000) != 0
}

unsafe extern "system" fn hook_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 {
        let kb = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
        // Ignore our own synthetic input (paste chords, typed text).
        if (kb.flags.0 & LLKHF_INJECTED.0) == 0 {
            let msg = wparam.0 as u32;
            let down = msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN;
            let up = msg == WM_KEYUP || msg == WM_SYSKEYUP;
            if (down || up) && on_key(kb.vkCode, down) {
                return LRESULT(1);
            }
        }
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

/// Returns whether to swallow the key.
fn on_key(vk: u32, down: bool) -> bool {
    STATE.with(|st| {
        let mut guard = st.borrow_mut();
        let Some(state) = guard.as_mut() else { return false };
        // Autorepeat sends repeated key-downs; only the first counts.
        let change = if !down {
            state.held.release(vk);
            KeyChange::Release
        } else if state.held.press(vk) {
            KeyChange::Press
        } else {
            KeyChange::Repeat
        };
        let (chord, capturing, recording) = {
            let s = state.shared.lock();
            (s.chord.clone(), s.capturing, s.recording)
        };
        let completed = match change {
            KeyChange::Repeat => false,
            KeyChange::Press => {
                // Before the shortcut can fire: a key that has looked held
                // a while may have been let go unseen (e.g. on the secure
                // desktop of a UAC prompt).
                let HookState { held, filter, .. } = &mut *state;
                held.drop_stale(STALE_KEY_AGE, |k| filter.is_dropped(k) || key_is_down(k));
                state.track(Some(vk), chord.as_ref(), capturing)
            }
            KeyChange::Release => state.track(None, chord.as_ref(), capturing),
        };
        let input = FilterInput {
            chord: chord.as_ref(),
            completed_chord: completed,
            recording,
            capturing,
        };
        state.filter.should_drop(&WinKeys, vk, change, &input)
    })
}

/// Every few seconds: forget keys Windows says are up, so a shortcut whose
/// release was missed still ends.
fn sweep() {
    STATE.with(|st| {
        let mut guard = st.borrow_mut();
        let Some(state) = guard.as_mut() else { return };
        let HookState { held, filter, .. } = &mut *state;
        if held.drop_stale(STALE_KEY_AGE, |k| filter.is_dropped(k) || key_is_down(k)).is_empty() {
            return;
        }
        let (chord, capturing) = {
            let s = state.shared.lock();
            (s.chord.clone(), s.capturing)
        };
        state.track(None, chord.as_ref(), capturing);
    });
}

impl HookState {
    /// Runs the tracker; returns whether this press completed the shortcut.
    fn track(&mut self, newly: Option<u32>, chord: Option<&Chord>, capturing: bool) -> bool {
        let (events, _mods) = self.tracker.update(&WinKeys, &self.held.keys, newly, chord, capturing);
        if events.iter().any(|e| matches!(e, HotkeyEvent::Captured(_))) {
            self.shared.lock().capturing = false;
        }
        let completed = events.contains(&HotkeyEvent::Down);
        for ev in events {
            if ev == HotkeyEvent::Down && chord.is_some_and(|c| c.mods.contains(Mods::SUPER)) {
                suppress_start_menu();
            }
            let _ = self.events.send(ev);
        }
        completed
    }
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

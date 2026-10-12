//! Global shortcut via a listen-only Quartz event tap. Requires
//! Accessibility (or Input Monitoring, which also allows listening).

use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;

use core_foundation::base::TCFType;
use core_foundation::mach_port::CFMachPortRef;
use core_foundation::runloop::{CFRunLoop, kCFRunLoopCommonModes};
use core_graphics::event::{
    CGEventFlags, CGEventTap, CGEventTapLocation, CGEventTapOptions, CGEventTapPlacement,
    CGEventType, CallbackResult, EventField,
};
use crossbeam_channel::Sender;
use parking_lot::Mutex;

use super::keycodes::{MODIFIERS, MacKeys};
use crate::hotkey::{Chord, ChordTracker, HotkeyEvent, KeyMap, Mods};

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGEventTapEnable(tap: CFMachPortRef, enable: bool);
}

#[derive(Default)]
struct Shared {
    chord: Option<Chord>,
    capturing: bool,
    held_mods: Mods,
}

struct TapState {
    pressed: HashSet<u32>,
    tracker: ChordTracker,
}

pub struct MacHotkeys {
    shared: Arc<Mutex<Shared>>,
}

impl MacHotkeys {
    pub fn spawn(chord: Chord, events: Sender<HotkeyEvent>) -> anyhow::Result<Self> {
        // Checked first: creating a tap without access pops a system prompt
        // before the setup assistant has explained anything.
        if !super::permissions::accessibility() && !super::permissions::input_monitoring() {
            anyhow::bail!("Accessibility permission is needed");
        }
        let shared = Arc::new(Mutex::new(Shared {
            chord: Some(chord),
            ..Default::default()
        }));
        let (ready_tx, ready_rx) = crossbeam_channel::bounded::<Result<(), String>>(1);
        let s = shared.clone();
        thread::Builder::new()
            .name("bars-hotkeys".into())
            .spawn(move || {
                let state = Mutex::new(TapState {
                    pressed: HashSet::new(),
                    tracker: ChordTracker::default(),
                });
                // The tap's port, for switching it back on from its callback.
                let port = Arc::new(AtomicUsize::new(0));
                let tap_port = port.clone();
                let tap = CGEventTap::new(
                    CGEventTapLocation::Session,
                    CGEventTapPlacement::HeadInsertEventTap,
                    CGEventTapOptions::ListenOnly,
                    vec![CGEventType::KeyDown, CGEventType::KeyUp, CGEventType::FlagsChanged],
                    move |_proxy, etype, event| {
                        // macOS switches taps off when a callback is slow, and
                        // while a password field has secure input (System
                        // Settings asks for one when toggling permissions).
                        // Without this the shortcut stays dead until restart.
                        if matches!(etype, CGEventType::TapDisabledByTimeout | CGEventType::TapDisabledByUserInput) {
                            log::info!("keyboard event tap was disabled by macOS ({etype:?}); re-enabling");
                            let mut st = state.lock();
                            // Keys may have changed while we weren't listening.
                            st.pressed.clear();
                            st.tracker = ChordTracker::default();
                            let p = tap_port.load(Ordering::Acquire);
                            if p != 0 {
                                unsafe { CGEventTapEnable(p as CFMachPortRef, true) };
                            }
                            return CallbackResult::Keep;
                        }
                        let code = event.get_integer_value_field(EventField::KEYBOARD_EVENT_KEYCODE) as u32;
                        let repeat = event.get_integer_value_field(EventField::KEYBOARD_EVENT_AUTOREPEAT) != 0;
                        let flags = event.get_flags();
                        let mut st = state.lock();
                        let newly = match etype {
                            CGEventType::KeyDown if !repeat => st.pressed.insert(code).then_some(code),
                            CGEventType::KeyDown => return CallbackResult::Keep,
                            CGEventType::KeyUp => {
                                st.pressed.remove(&code);
                                None
                            }
                            // Caps Lock also reports through here, but only when it
                            // toggles: tracked as a key it would look held while
                            // it's on, blocking the shortcut and shortcut recording.
                            CGEventType::FlagsChanged if MacKeys.modifier_of(code).is_none() => {
                                return CallbackResult::Keep;
                            }
                            CGEventType::FlagsChanged => {
                                // A modifier changed; which way is told by the flags.
                                let was_down = st.pressed.contains(&code);
                                let down = modifier_down(code, flags, was_down);
                                if down {
                                    st.pressed.insert(code);
                                } else {
                                    st.pressed.remove(&code);
                                }
                                sync_modifiers(&mut st.pressed, flags);
                                (down && !was_down).then_some(code)
                            }
                            _ => return CallbackResult::Keep,
                        };
                        let (chord, capturing) = {
                            let sh = s.lock();
                            (sh.chord.clone(), sh.capturing)
                        };
                        let TapState { pressed, tracker } = &mut *st;
                        let (events_out, mods) = tracker.update(&MacKeys, pressed, newly, chord.as_ref(), capturing);
                        {
                            let mut sh = s.lock();
                            sh.held_mods = mods;
                            if events_out.iter().any(|e| matches!(e, HotkeyEvent::Captured(_))) {
                                sh.capturing = false;
                            }
                        }
                        for ev in events_out {
                            let _ = events.send(ev);
                        }
                        CallbackResult::Keep
                    },
                );
                let tap = match tap {
                    Ok(t) => t,
                    Err(()) => {
                        let _ = ready_tx.send(Err("could not create the keyboard event tap".into()));
                        return;
                    }
                };
                port.store(tap.mach_port().as_concrete_TypeRef() as usize, Ordering::Release);
                let Ok(source) = tap.mach_port().create_runloop_source(0) else {
                    let _ = ready_tx.send(Err("could not attach the event tap".into()));
                    return;
                };
                CFRunLoop::get_current().add_source(&source, unsafe { kCFRunLoopCommonModes });
                tap.enable();
                let _ = ready_tx.send(Ok(()));
                CFRunLoop::run_current();
            })?;
        ready_rx
            .recv()
            .map_err(|_| anyhow::anyhow!("hotkey thread exited"))?
            .map_err(|e| anyhow::anyhow!(e))?;
        Ok(MacHotkeys { shared })
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

/// The event flag for a kind of modifier (either side).
fn flag_for(m: Mods) -> CGEventFlags {
    if m == Mods::FN {
        // Also set while arrow/function keys are held, but Fn is only ever
        // added from its own key code, so that can't fake a press.
        CGEventFlags::CGEventFlagSecondaryFn
    } else if m == Mods::SUPER {
        CGEventFlags::CGEventFlagCommand
    } else if m == Mods::SHIFT {
        CGEventFlags::CGEventFlagShift
    } else if m == Mods::ALT {
        CGEventFlags::CGEventFlagAlternate
    } else {
        CGEventFlags::CGEventFlagControl
    }
}

/// Whether the modifier `code` is down after a flags-changed event for it.
///
/// Keyboard events carry per-key bits (left/right Control, Shift, Option,
/// Command), which say exactly. Synthetic events may only carry the
/// either-side flag; then a change while it's set means this key went down,
/// unless it already was.
fn modifier_down(code: u32, flags: CGEventFlags, was_down: bool) -> bool {
    // (this key's bit, both sides' bits), from IOLLEvent.h (NX_DEVICE*KEYMASK).
    let (key_bit, kind_bits): (u64, u64) = match code {
        0x3B => (0x0001, 0x2001), // left Control
        0x3E => (0x2000, 0x2001), // right Control
        0x38 => (0x0002, 0x0006), // left Shift
        0x3C => (0x0004, 0x0006), // right Shift
        0x3A => (0x0020, 0x0060), // left Option
        0x3D => (0x0040, 0x0060), // right Option
        0x37 => (0x0008, 0x0018), // left Command
        0x36 => (0x0010, 0x0018), // right Command
        _ => (0, 0),
    };
    let bits = flags.bits();
    if bits & kind_bits != 0 {
        return bits & key_bit != 0;
    }
    let kind = MODIFIERS.iter().find(|(c, _)| *c == code).map(|(_, m)| *m);
    kind.is_some_and(|m| flags.contains(flag_for(m))) && !was_down
}

/// Drop modifier keys the flags say are no longer held (e.g. a release that
/// happened while the tap wasn't listening).
fn sync_modifiers(pressed: &mut HashSet<u32>, flags: CGEventFlags) {
    for (code, m) in MODIFIERS {
        if !flags.contains(flag_for(*m)) {
            pressed.remove(code);
        }
    }
}

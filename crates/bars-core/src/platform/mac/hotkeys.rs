//! Global shortcut via a listen-only Quartz event tap. Requires the Input
//! Monitoring permission.

use std::collections::HashSet;
use std::sync::Arc;
use std::thread;

use core_foundation::runloop::{CFRunLoop, kCFRunLoopCommonModes};
use core_graphics::event::{
    CGEventFlags, CGEventTap, CGEventTapLocation, CGEventTapOptions, CGEventTapPlacement,
    CGEventType, CallbackResult, EventField,
};
use crossbeam_channel::Sender;
use parking_lot::Mutex;

use super::keycodes::{MODIFIERS, MacKeys};
use crate::hotkey::{Chord, ChordTracker, HotkeyEvent, Mods};

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
        if !super::permissions::can_listen() {
            anyhow::bail!("Input Monitoring permission is needed");
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
                let tap = CGEventTap::new(
                    CGEventTapLocation::Session,
                    CGEventTapPlacement::HeadInsertEventTap,
                    CGEventTapOptions::ListenOnly,
                    vec![CGEventType::KeyDown, CGEventType::KeyUp, CGEventType::FlagsChanged],
                    move |_proxy, etype, event| {
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
                            CGEventType::FlagsChanged => {
                                // A modifier changed; which way is told by the flags.
                                let down = !st.pressed.contains(&code);
                                if down {
                                    st.pressed.insert(code);
                                } else {
                                    st.pressed.remove(&code);
                                }
                                sync_modifiers(&mut st.pressed, flags);
                                (down && st.pressed.contains(&code)).then_some(code)
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

/// Drop modifier keys the flags say are no longer held (e.g. a release that
/// happened while the tap wasn't listening).
fn sync_modifiers(pressed: &mut HashSet<u32>, flags: CGEventFlags) {
    for (code, m) in MODIFIERS {
        let mask = if *m == Mods::SUPER {
            CGEventFlags::CGEventFlagCommand
        } else if *m == Mods::SHIFT {
            CGEventFlags::CGEventFlagShift
        } else if *m == Mods::ALT {
            CGEventFlags::CGEventFlagAlternate
        } else {
            CGEventFlags::CGEventFlagControl
        };
        if !flags.contains(mask) {
            pressed.remove(code);
        }
    }
}

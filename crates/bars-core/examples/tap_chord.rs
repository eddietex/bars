//! Dev helper: taps Ctrl+Super through a throwaway virtual keyboard to
//! exercise the hotkey path. usage: tap_chord <hold_ms> [taps] [gap_ms]
use std::{thread::sleep, time::Duration};
use evdev::{AttributeSet, InputEvent, KeyCode, EventType, uinput::VirtualDevice};

fn main() -> std::io::Result<()> {
    let mut a = std::env::args().skip(1).map(|v| v.parse::<u64>().unwrap());
    let (hold, taps, gap) = (a.next().unwrap_or(100), a.next().unwrap_or(1), a.next().unwrap_or(150));
    let mut keys = AttributeSet::<KeyCode>::new();
    for code in 1..=248u16 { keys.insert(KeyCode::new(code)); } // full set: udev must see a keyboard
    let mut dev = VirtualDevice::builder()?.name("bars-test-keyboard").with_keys(&keys)?.build()?;
    sleep(Duration::from_millis(3500)); // let the app's rescan pick the device up
    let ev = |k: KeyCode, v| InputEvent::new(EventType::KEY.0, k.code(), v);
    for i in 0..taps {
        if i > 0 { sleep(Duration::from_millis(gap)); }
        dev.emit(&[ev(KeyCode::KEY_LEFTCTRL, 1)])?;
        dev.emit(&[ev(KeyCode::KEY_LEFTMETA, 1)])?;
        sleep(Duration::from_millis(hold));
        dev.emit(&[ev(KeyCode::KEY_LEFTMETA, 0)])?;
        dev.emit(&[ev(KeyCode::KEY_LEFTCTRL, 0)])?;
    }
    sleep(Duration::from_millis(300));
    Ok(())
}

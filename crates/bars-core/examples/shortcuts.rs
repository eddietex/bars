//! Prints what the shortcut listener sees, without dictating: the shortcuts
//! given (or the config's), and with `--record` the next one pressed.
//! usage: cargo run -p bars-core --example shortcuts -- [--record] [--secs N] [Ctrl+Super Mouse4 ...]

use std::time::{Duration, Instant};

use bars_core::config::{Config, HotkeyBackendKind};
use bars_core::hotkey::Chord;
use bars_core::platform::Hotkeys;

fn main() {
    env_logger::init();
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let record = take_flag(&mut args, "--record");
    let secs = match args.iter().position(|a| a == "--secs") {
        Some(i) => {
            args.remove(i);
            args.remove(i).parse().expect("--secs N")
        }
        None => 30,
    };
    let chords: Vec<Chord> = if args.is_empty() {
        Config::load_or_default(&bars_core::paths::config_file()).hotkey.chords
    } else {
        args.iter().map(|a| a.parse().expect("a shortcut like Ctrl+Super or Mouse4")).collect()
    };
    let names: Vec<String> = chords.iter().map(Chord::label).collect();
    println!("shortcuts: {}", names.join(", "));
    let (tx, rx) = crossbeam_channel::unbounded();
    let hotkeys = Hotkeys::start(HotkeyBackendKind::Auto, chords, tx);
    if let Some(e) = hotkeys.error() {
        eprintln!("no shortcut listener: {e}");
        std::process::exit(1);
    }
    if record {
        hotkeys.set_capture(true);
        println!("press a shortcut to record…");
    }
    let start = Instant::now();
    let end = start + Duration::from_secs(secs);
    while let Ok(ev) = rx.recv_timeout(end.saturating_duration_since(Instant::now())) {
        println!("{:>6} ms  {ev:?}", start.elapsed().as_millis());
    }
}

fn take_flag(args: &mut Vec<String>, flag: &str) -> bool {
    let found = args.iter().position(|a| a == flag);
    if let Some(i) = found {
        args.remove(i);
    }
    found.is_some()
}

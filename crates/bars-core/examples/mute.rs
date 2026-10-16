//! Mutes other audio for a few seconds, as Bars does while you dictate, then
//! turns it back on; play some music to hear it. `--kill` dies while
//! muted instead, to see that a crash doesn't leave things muted (the next
//! run, or Bars' next start, turns back on what's left).
//! usage: RUST_LOG=info cargo run -p bars-core --example mute -- [--kill] [secs]

use std::time::Duration;

use bars_core::platform::mute::Muter;

fn main() {
    env_logger::init();
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let kill = args.iter().position(|a| a == "--kill").map(|i| args.remove(i)).is_some();
    let secs: f32 = args.first().map_or(5.0, |s| s.parse().expect("seconds"));
    let muter = Muter::spawn();
    println!("muting other audio for {secs} s…");
    muter.mute(Duration::ZERO);
    std::thread::sleep(Duration::from_secs_f32(secs));
    if kill {
        println!("dying while muted");
        std::process::abort();
    }
    muter.restore();
    println!("back on");
}

//! Polishes dictations with Spokenly Cleanup 1 and times it, downloading
//! the model if needed. One dictation per line, from the arguments or
//! stdin; `before|text|after` gives the text around the cursor.
//! usage: cargo run --release -p bars-core --example polish -- "um so i think we should uh ship it"

use std::io::BufRead;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Instant;

use bars_core::text::Surroundings;
use bars_core::{models, polish};

fn main() -> anyhow::Result<()> {
    let info = &models::POLISH;
    if !info.is_installed() {
        eprintln!("downloading {} ...", info.name);
        models::download(info, Arc::new(AtomicBool::new(false)), &mut |_, _| {})?;
    }
    let t0 = Instant::now();
    let model = polish::Model::load(&info.dir().join(polish::MODEL_FILE))?;
    let _ = model.rewrite("Hello there.", None, None)?;
    eprintln!("loaded in {} ms", t0.elapsed().as_millis());

    let args: Vec<String> = std::env::args().skip(1).collect();
    let lines: Vec<String> = if args.is_empty() {
        std::io::stdin().lock().lines().map_while(Result::ok).filter(|l| !l.trim().is_empty()).collect()
    } else {
        args
    };
    let mut times = Vec::new();
    for line in lines {
        let parts: Vec<&str> = line.split('|').collect();
        let (field, text) = match parts.as_slice() {
            [before, text, after] => (Some(Surroundings { before: before.to_string(), selected: String::new(), after: after.to_string() }), *text),
            _ => (None, line.as_str()),
        };
        let t = Instant::now();
        let out = model.rewrite(text, None, field.as_ref())?;
        let ms = t.elapsed().as_millis();
        times.push(ms);
        let verdict = match &out {
            Some(o) if polish::acceptable(text, o) => "ok",
            Some(_) => "REJECTED",
            None => "TIMEOUT",
        };
        println!("{ms:>5} ms  {verdict:<8}  {text}\n                   → {}", out.unwrap_or_default());
    }
    times.sort();
    if !times.is_empty() {
        println!("median {} ms, max {} ms", times[times.len() / 2], times[times.len() - 1]);
    }
    Ok(())
}

//! Windows/macOS remote control and single instance: a local socket (named
//! pipe on Windows, Unix socket on macOS) carrying one command per line,
//! mirroring the Linux D-Bus interface.

use std::io::{BufRead, BufReader, Write};

use interprocess::local_socket::{GenericFilePath, GenericNamespaced, ListenerOptions, Name, Stream, prelude::*};

use crate::shell::ShellCommand;

fn name() -> std::io::Result<Name<'static>> {
    if GenericNamespaced::is_supported() {
        "xyz.hink.Bars.sock".to_ns_name::<GenericNamespaced>().map(Name::into_owned)
    } else {
        // macOS: a per-user socket file.
        let user = std::env::var("USER").unwrap_or_default();
        std::env::temp_dir()
            .join(format!("xyz.hink.Bars-{user}.sock"))
            .to_fs_name::<GenericFilePath>()
            .map(Name::into_owned)
    }
}

/// Marks this process as the running instance while alive.
pub struct Instance;

/// Becomes the primary instance (and starts serving commands), or returns
/// `None` if Bars is already running.
pub fn claim(tx: async_channel::Sender<ShellCommand>) -> anyhow::Result<Option<Instance>> {
    // Someone answering means another instance is running.
    if Stream::connect(name()?).is_ok() {
        return Ok(None);
    }
    let listener = ListenerOptions::new().name(name()?).try_overwrite(true).create_sync()?;
    std::thread::Builder::new()
        .name("bars-ipc".into())
        .spawn(move || {
            for conn in listener.incoming().filter_map(Result::ok) {
                let mut line = String::new();
                if BufReader::new(conn).read_line(&mut line).is_err() {
                    continue;
                }
                let cmd = match line.trim() {
                    "Toggle" => ShellCommand::Toggle,
                    "Cancel" => ShellCommand::Cancel,
                    "PasteLast" => ShellCommand::PasteLast,
                    "ShowSettings" => ShellCommand::ShowSettings(None),
                    "Quit" => ShellCommand::Quit,
                    _ => continue,
                };
                let _ = tx.send_blocking(cmd);
            }
        })?;
    Ok(Some(Instance))
}

/// Sends a command to the running instance.
pub fn call(method: &str) -> anyhow::Result<()> {
    let mut stream = Stream::connect(name()?)?;
    stream.write_all(format!("{method}\n").as_bytes())?;
    stream.flush()?;
    Ok(())
}

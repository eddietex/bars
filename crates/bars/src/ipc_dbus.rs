//! D-Bus service: single-instance handling and remote control.
//!
//! `bars --toggle` (bindable to any compositor shortcut) talks to the
//! running instance through this interface.

use crate::shell::ShellCommand;

pub const BUS_NAME: &str = "xyz.hink.Bars";
pub const OBJECT_PATH: &str = "/xyz/hink/Bars";

struct Service {
    tx: async_channel::Sender<ShellCommand>,
}

#[zbus::interface(name = "xyz.hink.Bars1")]
impl Service {
    /// Start hands-free dictation, or stop and insert.
    async fn toggle(&self) {
        let _ = self.tx.send(ShellCommand::Toggle).await;
    }

    async fn cancel(&self) {
        let _ = self.tx.send(ShellCommand::Cancel).await;
    }

    async fn paste_last(&self) {
        let _ = self.tx.send(ShellCommand::PasteLast).await;
    }

    async fn show_settings(&self) {
        let _ = self.tx.send(ShellCommand::ShowSettings(None)).await;
    }

    /// Install the update found, or look for one.
    async fn update(&self) {
        let _ = self.tx.send(ShellCommand::Update).await;
    }

    async fn quit(&self) {
        let _ = self.tx.send(ShellCommand::Quit).await;
    }
}

/// Tries to become the primary instance. Returns the connection (keep it
/// alive) or `None` if another instance already owns the name.
pub fn claim(tx: async_channel::Sender<ShellCommand>) -> anyhow::Result<Option<zbus::blocking::Connection>> {
    use zbus::fdo::{RequestNameFlags, RequestNameReply};
    let conn = zbus::blocking::connection::Builder::session()?
        .serve_at(OBJECT_PATH, Service { tx })?
        .build()?;
    // Request the name ourselves: the builder's `.name()` treats "already
    // owned by someone else" as success.
    let reply = match conn.request_name_with_flags(BUS_NAME, RequestNameFlags::DoNotQueue.into()) {
        Ok(reply) => reply,
        Err(zbus::Error::NameTaken) => RequestNameReply::Exists,
        Err(e) => return Err(e.into()),
    };
    log::debug!("RequestName({BUS_NAME}) -> {reply:?}");
    match reply {
        RequestNameReply::PrimaryOwner | RequestNameReply::AlreadyOwner => Ok(Some(conn)),
        RequestNameReply::Exists | RequestNameReply::InQueue => Ok(None),
    }
}

/// Calls a method on the running instance.
pub fn call(method: &str) -> anyhow::Result<()> {
    let conn = zbus::blocking::Connection::session()?;
    conn.call_method(Some(BUS_NAME), OBJECT_PATH, Some("xyz.hink.Bars1"), method, &())?;
    Ok(())
}

//! What has keyboard focus when a dictation starts: the app, its window
//! title and the text around the cursor, read through each platform's
//! accessibility API. Smart spacing fits the dictation into that text; the
//! app, title and its child processes tell whether a terminal is running
//! Claude Code or Codex.
//!
//! Field text is sensitive: it lives in memory for one dictation and is
//! never logged or saved.

use std::sync::OnceLock;
use std::thread;
use std::time::Duration;

use crossbeam_channel::{Receiver, Sender, bounded};

use crate::text::{Agent, Surroundings};

/// Characters read on each side of the cursor for smart spacing.
pub const CONTEXT_CHARS: usize = 240;

#[derive(Clone, Debug, Default)]
pub struct Focus {
    /// Bundle identifier (macOS) or lowercase executable name.
    pub app: Option<String>,
    pub pid: Option<u32>,
    pub window_title: Option<String>,
    /// The text around the cursor. None for password fields, or when the
    /// app doesn't expose it.
    pub field: Option<Surroundings>,
    pub agent: Option<Agent>,
}

impl Focus {
    /// A terminal has focus. Its "text field" is the screen, and the cursor
    /// position it reports can't be trusted (Ghostty: always the top).
    pub fn in_terminal(&self) -> bool {
        self.app.as_deref().is_some_and(is_terminal)
    }

    /// For logs: what was found, without any of the text.
    pub fn summary(&self) -> String {
        format!(
            "app {:?}, field {}, agent {:?}",
            self.app.as_deref().unwrap_or("?"),
            match &self.field {
                Some(f) => format!("{}+{}+{} chars", f.before.chars().count(), f.selected.chars().count(), f.after.chars().count()),
                None => "unreadable".into(),
            },
            self.agent
        )
    }
}

type Job = (Option<usize>, Sender<Focus>);

/// Starts reading what's focused, and with `field_chars` that many
/// characters of the field on each side of the cursor. Captures run one at
/// a time on a long-lived worker, so one stuck inside a hung app only makes
/// later captures come back empty; it never blocks the caller.
pub fn start_capture(field_chars: Option<usize>) -> Receiver<Focus> {
    static WORKER: OnceLock<Option<Sender<Job>>> = OnceLock::new();
    let worker = WORKER.get_or_init(|| {
        let (tx, rx) = bounded::<Job>(1);
        thread::Builder::new()
            .name("bars-focus".into())
            .spawn(move || {
                for (field_chars, reply) in rx {
                    let mut focus = read_focus(field_chars);
                    focus.agent = coding_agent(&focus);
                    log::debug!("focus: {}", focus.summary());
                    let _ = reply.send(focus);
                }
            })
            .ok()
            .map(|_| tx)
    });
    let (tx, rx) = bounded(1);
    match worker {
        Some(w) if w.try_send((field_chars, tx.clone())).is_ok() => {}
        _ => {
            log::debug!("focus: the previous capture is still running");
            let _ = tx.send(Focus::default());
        }
    }
    rx
}

/// Reads what's focused, giving up after `timeout`.
pub fn capture(field_chars: Option<usize>, timeout: Duration) -> Focus {
    start_capture(field_chars).recv_timeout(timeout).unwrap_or_default()
}

fn read_focus(field_chars: Option<usize>) -> Focus {
    #[cfg(target_os = "macos")]
    {
        super::mac::ax::read_focus(field_chars)
    }
    #[cfg(target_os = "windows")]
    {
        super::win::uia::read_focus(field_chars)
    }
    #[cfg(target_os = "linux")]
    {
        super::linux::atspi::read_focus(field_chars)
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        let _ = field_chars;
        Focus::default()
    }
}

#[cfg(target_os = "macos")]
const TERMINALS: &[&str] = &[
    "com.apple.Terminal",
    "com.googlecode.iterm2",
    "dev.warp.Warp-Stable",
    "com.mitchellh.ghostty",
    "net.kovidgoyal.kitty",
    "io.alacritty",
    "org.alacritty",
    "co.zeit.hyper",
    "com.github.wez.wezterm",
    "org.tabby",
];
/// Executable names (prefixes).
#[cfg(target_os = "windows")]
const TERMINALS: &[&str] = &[
    "windowsterminal", "cmd", "powershell", "pwsh", "warp", "alacritty", "hyper", "kitty", "wezterm", "tabby",
    "openconsole", "conhost",
];
/// Executable names (prefixes; /proc truncates them to 15 characters).
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
const TERMINALS: &[&str] = &[
    "konsole", "gnome-terminal", "ptyxis", "kgx", "tilix", "terminator", "xfce4-terminal", "kitty", "alacritty",
    "foot", "wezterm", "ghostty", "yakuake", "terminology", "guake", "xterm", "lxterminal", "mate-terminal",
    "qterminal", "blackbox", "cool-retro-term", "rio", "warp", "tabby",
];

pub(super) fn is_terminal(app: &str) -> bool {
    if cfg!(target_os = "macos") {
        TERMINALS.contains(&app)
    } else {
        TERMINALS.iter().any(|t| app.starts_with(t))
    }
}

/// Claude Code or Codex in the focused terminal: named in the window title,
/// or running below the terminal's process.
fn coding_agent(focus: &Focus) -> Option<Agent> {
    let Some(app) = focus.app.as_deref() else {
        // Linux apps without accessibility (most terminals) can't be told
        // apart. Splitting a paste is harmless elsewhere, so split whenever
        // an agent is running at all.
        return if cfg!(target_os = "linux") { super::procs::agent_running() } else { None };
    };
    if !is_terminal(app) {
        return None;
    }
    let title = focus.window_title.as_deref().unwrap_or("").to_lowercase();
    if title.contains("claude") {
        return Some(Agent::ClaudeCode);
    }
    if title.contains("codex") {
        return Some(Agent::Codex);
    }
    focus.pid.and_then(super::procs::agent_below)
}

/// Which agent a process is, from its executable name and arguments
/// (`node .../claude`, `bun .../codex.js`).
pub(super) fn agent_from_process(name: &str, args: impl Fn() -> Vec<String>) -> Option<Agent> {
    let stem = |s: &str| {
        let file = s.rsplit(['/', '\\']).next().unwrap_or(s).to_ascii_lowercase();
        file.strip_suffix(".exe").map(str::to_string).unwrap_or(file)
    };
    let of = |s: &str| match stem(s).as_str() {
        "claude" | "claude.js" | "cli.js" if s.contains("claude") => Some(Agent::ClaudeCode),
        "codex" | "codex.js" => Some(Agent::Codex),
        _ => None,
    };
    if let Some(a) = of(name) {
        return Some(a);
    }
    if matches!(stem(name).as_str(), "node" | "bun" | "deno") {
        return args().iter().skip(1).take(3).find_map(|a| of(a));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_agent_processes() {
        let none = Vec::new;
        assert_eq!(agent_from_process("claude", none), Some(Agent::ClaudeCode));
        assert_eq!(agent_from_process("codex.exe", none), Some(Agent::Codex));
        assert_eq!(agent_from_process("zsh", none), None);
        let args = || vec!["node".into(), "/usr/lib/node_modules/@anthropic-ai/claude-code/cli.js".into()];
        assert_eq!(agent_from_process("node", args), Some(Agent::ClaudeCode));
        let args = || vec!["node".into(), "/home/me/.npm/bin/codex".into()];
        assert_eq!(agent_from_process("node", args), Some(Agent::Codex));
        let args = || vec!["node".into(), "server.js".into()];
        assert_eq!(agent_from_process("node", args), None);
    }
}

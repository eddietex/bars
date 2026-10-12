//! Bars: local voice dictation that types into any app.

mod autostart;
mod brand;
#[cfg(target_os = "linux")]
mod ipc;
mod onboarding;
mod overlay;
mod settings;
mod shell;
mod state;
mod theme;
#[cfg(target_os = "linux")]
mod tray;
mod widgets;

use std::borrow::Cow;

use gpui_kit::*;
use bars_core::config::Config;
use bars_core::{controller, paths};

use crate::shell::{Launch, Shell, ShellCommand};
use crate::state::AppState;

gpui_kit::assets::icon_assets!(
    AppIcons,
    [
        House, Keyboard, AudioLines, BookA, ListClock, ShieldCheck, X, Check, Copy, Trash, Sparkles,
        ArrowRight, Search, ChevronsUpDown, CircleCheck, CircleAlert, Zap, Mic, Pencil
    ]
);

struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if let Some(bytes) = brand::load_asset(path) {
            return Ok(Some(bytes));
        }
        if let Some(bytes) = AppIcons.load(path)? {
            return Ok(Some(bytes));
        }
        gpui_kit::assets::Assets.load(path)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut paths = gpui_kit::assets::Assets.list(path)?;
        paths.extend(AppIcons.list(path)?);
        paths.sort();
        paths.dedup();
        Ok(paths)
    }
}

const USAGE: &str = "Usage: bars [--background | --toggle | --cancel | --paste-last | --settings | --quit | --doctor]";

fn main() {
    bars_core::audio::prepare_environment();
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info,ort=warn,naga=warn,wgpu=warn,blade_graphics=warn,transcribe_rs=warn,zbus=error,tracing::span=off,pulseaudio::client::reactor=off"))
        .init();

    let arg = std::env::args().nth(1);
    let remote = match arg.as_deref() {
        None | Some("--background") => None,
        Some("--toggle") => Some("Toggle"),
        Some("--cancel") => Some("Cancel"),
        Some("--paste-last") => Some("PasteLast"),
        Some("--settings") => Some("ShowSettings"),
        Some("--quit") => Some("Quit"),
        Some("-h" | "--help") => {
            println!("{USAGE}");
            return;
        }
        Some("--doctor") => {
            print!("{}", bars_core::platform::doctor_report());
            return;
        }
        Some(other) => {
            eprintln!("unknown argument {other}\n{USAGE}");
            std::process::exit(2);
        }
    };
    let background = arg.as_deref() == Some("--background");

    let (shell_tx, shell_rx) = async_channel::unbounded::<ShellCommand>();

    // Single instance: if Bars is already running, forward and exit.
    #[cfg(target_os = "linux")]
    let _bus = match ipc::claim(shell_tx.clone()) {
        Ok(Some(conn)) => Some(conn),
        Ok(None) => {
            let method = remote.unwrap_or("ShowSettings");
            if let Err(e) = ipc::call(method) {
                eprintln!("could not reach the running Bars: {e:#}");
                std::process::exit(1);
            }
            return;
        }
        Err(e) => {
            log::warn!("D-Bus unavailable ({e:#}); continuing without single-instance support");
            None
        }
    };
    if remote.is_some_and(|m| m != "ShowSettings") {
        eprintln!("Bars isn't running.");
        std::process::exit(1);
    }

    let config = Config::load_or_default(&paths::config_file());
    let (ui_tx, ui_rx) = async_channel::bounded(1024);
    let handle = controller::spawn(config.clone(), ui_tx);

    gpui_kit::application()
        .with_assets(Assets)
        // Linux has a tray, so closing windows keeps Bars running. Elsewhere
        // (no tray yet) use the platform default so the app can't linger
        // invisibly.
        .with_quit_mode(if cfg!(target_os = "linux") { QuitMode::Explicit } else { QuitMode::Default })
        .run(move |cx| {
            gpui_kit::init(cx);
            let fonts: Vec<Cow<'static, [u8]>> = vec![
                Cow::Borrowed(include_bytes!("../assets/fonts/Inter-Regular.ttf")),
                Cow::Borrowed(include_bytes!("../assets/fonts/Inter-Medium.ttf")),
                Cow::Borrowed(include_bytes!("../assets/fonts/Inter-SemiBold.ttf")),
                Cow::Borrowed(include_bytes!("../assets/fonts/Inter-Bold.ttf")),
            ];
            if let Err(e) = cx.text_system().add_fonts(fonts) {
                log::warn!("could not load bundled fonts: {e}");
            }
            theme::set_pref(config.ui.theme, cx);
            theme::apply(theme::wants_dark(config.ui.theme, cx), cx);
            autostart::sync(config.ui.launch_at_login);

            let state = cx.new(|cx| AppState::new(handle, config.clone(), cx));
            state::pump(state.clone(), ui_rx, cx);

            // Keep autostart in sync with the setting.
            let mut last_autostart = config.ui.launch_at_login;
            cx.observe(&state, move |state, cx| {
                let want = state.read(cx).config.ui.launch_at_login;
                if want != last_autostart {
                    last_autostart = want;
                    autostart::sync(want);
                }
            })
            .detach();

            cx.set_global(shell::ShellSender(shell_tx.clone()));
            let shell = cx.new(|cx| Shell::new(state.clone(), shell_rx, shell_tx.clone(), cx));
            let launch = if background { Launch::Background } else { Launch::Interactive };
            shell.update(cx, |s, cx| s.startup(launch, cx));
            // Keep the shell alive for the life of the app.
            std::mem::forget(shell);
        });
}

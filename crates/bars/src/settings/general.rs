use gpui_kit::assets::IconName as I;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use bars_core::config::{InsertMethod, OverlayMode, PasteKeys, ThemeMode};
use bars_core::controller::Command;
use bars_core::hotkey::{Activation, Mods};

use super::{Page, SettingsView};

#[cfg(target_os = "macos")]
const MODIFIER_ONLY_HINT: &str = "Works in every app. Modifier-only shortcuts like Fn or Ctrl + Cmd never type anything.";
#[cfg(target_os = "windows")]
const MODIFIER_ONLY_HINT: &str = "Works in every app. Modifier-only shortcuts like Ctrl + Win never type anything.";
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
const MODIFIER_ONLY_HINT: &str = "Works in every app. Modifier-only shortcuts like Ctrl + Super never type anything.";

#[cfg(target_os = "linux")]
const SMART_SPACING_HINT: &str = "Reads the text around the cursor to add spaces, match capitalization and drop a final full stop mid-sentence. Works in apps with accessibility support (most GTK apps). Nothing is stored.";
#[cfg(not(target_os = "linux"))]
const SMART_SPACING_HINT: &str = "Reads the text around the cursor to add spaces, match capitalization and drop a final full stop mid-sentence. Nothing is stored.";
use crate::state::UpdateStatus;
use crate::theme::PaletteExt;
use crate::widgets::*;

impl SettingsView {
    pub fn render_general(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = cx.palette();
        let st = self.state.read(cx);
        let cfg = st.config.clone();
        let capturing = st.capturing;
        let capture_error = st.capture_error.clone();
        // The Globe key also opens the emoji picker/dictation unless told not to.
        let globe_hint = cfg!(target_os = "macos") && cfg.hotkey.chord.mods.contains(Mods::FN);
        let globe_link = div()
            .id("globe-settings")
            .text_color(p.accent_text)
            .cursor_pointer()
            .hover(|s| s.underline())
            .child("Keyboard settings")
            .on_click(cx.listener(|this, _, _, cx| {
                this.state.read(cx).send(Command::OpenUrl(
                    "x-apple.systempreferences:com.apple.Keyboard-Settings.extension".into(),
                ))
            }));
        let hotkeys_ok = st.hotkeys_active;
        let secure_input = st.secure_input.clone();
        let desktop_shortcut = st.desktop_shortcut;
        let shortcut_parts = st.shortcut_parts();
        let state = self.state.clone();

        let set = {
            let state = state.clone();
            move |cx: &mut App, f: Box<dyn FnOnce(&mut bars_core::config::Config)>| {
                state.update(cx, |s, cx| s.edit_config(cx, f));
            }
        };

        // --- Shortcut recorder ------------------------------------------
        let recorder = {
            let state = state.clone();
            h_row()
                .gap(px(10.))
                .child(if capturing {
                    h_row()
                        .h(px(34.))
                        .px(px(14.))
                        .rounded(px(8.))
                        .border_1()
                        .border_color(p.accent)
                        .bg(p.accent_soft)
                        .text_size(px(13.))
                        .text_color(p.accent_text)
                        .child("Press your shortcut…")
                        .into_any_element()
                } else {
                    keycaps(shortcut_parts, p, true).into_any_element()
                })
                .child(if capturing {
                    button("cancel-capture", "Cancel", ButtonKind::Ghost, p)
                        .on_click({
                            let state = state.clone();
                            move |_, _, cx| state.update(cx, |s, cx| s.stop_capture(cx))
                        })
                        .into_any_element()
                } else {
                    icon_label_button("change-shortcut", I::Pencil, "Change", ButtonKind::Accent, p)
                        .when(!hotkeys_ok, |el| el.opacity(0.5))
                        .on_click({
                            let state = state.clone();
                            // Without a running shortcut listener no keys would
                            // ever arrive: show what to allow instead.
                            let open_setup = cx.listener(|this, _: &ClickEvent, _, cx| this.go(Page::Setup, cx));
                            move |ev, window, cx| {
                                if !hotkeys_ok {
                                    open_setup(ev, window, cx);
                                    return;
                                }
                                state.update(cx, |s, cx| {
                                    if s.desktop_shortcut {
                                        s.send(Command::ConfigureShortcut);
                                    } else {
                                        s.start_capture(cx)
                                    }
                                })
                            }
                        })
                        .into_any_element()
                })
        };

        let activation = segmented(
            "activation",
            vec![
                (Activation::HoldOrDoubleTap, "Hold · double-tap"),
                (Activation::Hold, "Hold"),
                (Activation::Toggle, "Toggle"),
            ],
            cfg.hotkey.activation,
            p,
            {
                let set = set.clone();
                move |v, _, cx| set(cx, Box::new(move |c| c.hotkey.activation = v))
            },
        );

        // --- Microphone -------------------------------------------------
        let mic_options = st.mic_options();
        let mic_picker = picker(
            "mic",
            mic_options,
            cfg.audio.device.clone(),
            p,
            {
                let set = set.clone();
                move |v: Option<String>, cx: &mut App| set(cx, Box::new(move |c| c.audio.device = v))
            },
        );

        let paste_keys = segmented(
            "paste-keys",
            vec![
                (PasteKeys::ShiftInsert, "Shift+Insert"),
                (PasteKeys::CtrlV, "Ctrl+V"),
                (PasteKeys::CtrlShiftV, "Ctrl+Shift+V"),
            ],
            cfg.insertion.paste_keys,
            p,
            {
                let set = set.clone();
                move |v, _, cx| set(cx, Box::new(move |c| c.insertion.paste_keys = v))
            },
        );

        // --- Updates ----------------------------------------------------
        let update = st.update.clone();
        let update_status = match &update {
            UpdateStatus::Unknown => "Not checked yet.".to_string(),
            UpdateStatus::Checking => "Checking…".into(),
            UpdateStatus::UpToDate => "Up to date.".into(),
            UpdateStatus::Available(r) => format!("ver. {} is available.", r.version),
            UpdateStatus::Installing(r) => format!("Installing ver. {}…", r.version),
            UpdateStatus::InstallFailed(_, why) | UpdateStatus::Failed(why) => why.clone(),
        };
        let update_button = match &update {
            UpdateStatus::Available(_) => button("update", crate::shell::update_action(), ButtonKind::Primary, p),
            UpdateStatus::InstallFailed(..) => button("update", "Try again", ButtonKind::Primary, p),
            UpdateStatus::Checking | UpdateStatus::Installing(_) => {
                button("update", "Check now", ButtonKind::Secondary, p).opacity(0.5)
            }
            _ => button("update", "Check now", ButtonKind::Secondary, p),
        }
        .on_click({
            let state = state.clone();
            move |_, _, cx| {
                state.update(cx, |s, cx| match s.update {
                    UpdateStatus::Available(_) | UpdateStatus::InstallFailed(..) => s.install_update(cx),
                    _ => s.check_for_updates(true, cx),
                })
            }
        });
        let version = format!("ver. {}", env!("CARGO_PKG_VERSION"));

        v_col()
            .child(page_header("Shortcut & input", Some("How you start dictating and where the words go."), p))
            .child(
                section("Shortcut", p).child(rows(
                    p,
                    vec![
                        v_col()
                            .child(row(
                                "Dictation shortcut",
                                Some(if desktop_shortcut {
                                    "Managed by your desktop's shortcut settings. Allow direct keyboard access in Setup to use modifier-only shortcuts."
                                } else {
                                    MODIFIER_ONLY_HINT
                                }),
                                recorder,
                                p,
                            ))
                            .when_some(capture_error, |el, e| {
                                el.child(div().px(px(18.)).pb(px(12.)).text_size(px(12.5)).text_color(p.danger).child(e))
                            })
                            .when(globe_hint, |el| {
                                el.child(
                                    h_row()
                                        .px(px(18.))
                                        .pb(px(12.))
                                        .gap(px(6.))
                                        .flex_wrap()
                                        .text_size(px(12.5))
                                        .text_color(p.text_muted)
                                        .child("macOS also reacts to Fn (🌐). Set “Press 🌐 key to” to “Do Nothing” in")
                                        .child(globe_link),
                                )
                            })
                            .when_some(secure_input, |el, app| {
                                el.child(
                                    div()
                                        .px(px(18.))
                                        .pb(px(12.))
                                        .text_size(px(12.5))
                                        .text_color(p.warning)
                                        .child(format!(
                                            "{app} has Secure Keyboard Entry on, which hides your keys from Bars. \
                                             Turn it off there (Terminal: Terminal menu › Secure Keyboard Entry), or switch apps."
                                        )),
                                )
                            })
                            .when(!hotkeys_ok, |el| {
                                el.child(
                                    div()
                                        .px(px(18.))
                                        .pb(px(12.))
                                        .text_size(px(12.5))
                                        .text_color(p.warning)
                                        .child("Keyboard access is off — see Setup."),
                                )
                            })
                            .into_any_element(),
                        row(
                            "Activation",
                            Some("Hold to talk is fastest. Double-tap locks hands-free for long dictation."),
                            activation,
                            p,
                        )
                        .into_any_element(),
                    ],
                )),
            )
            .child(
                section("Microphone", p).child(rows(
                    p,
                    vec![
                        row("Input device", None, mic_picker, p).into_any_element(),
                        row(
                            "Keep microphone ready",
                            Some("Zero start delay and a short pre-roll so the first word is never clipped. The mic stays open in the background."),
                            small_switch("warm", cfg.audio.keep_warm, {
                                let set = set.clone();
                                move |v, _, cx| {
                                    let v = *v;
                                    set(cx, Box::new(move |c| c.audio.keep_warm = v))
                                }
                            }),
                            p,
                        )
                        .into_any_element(),
                        row(
                            "Sound cues",
                            Some("Soft tones when recording starts and stops."),
                            small_switch("sounds", cfg.audio.sounds, {
                                let set = set.clone();
                                move |v, _, cx| {
                                    let v = *v;
                                    set(cx, Box::new(move |c| c.audio.sounds = v))
                                }
                            }),
                            p,
                        )
                        .into_any_element(),
                    ],
                )),
            )
            .child(
                section("Inserting text", p).child(rows(
                    p,
                    vec![
                        row(
                            "Method",
                            Some(match cfg.insertion.method {
                                InsertMethod::Auto => "Pastes when the clipboard is available, otherwise types.",
                                InsertMethod::Paste => "Instant, handles any character; briefly uses the clipboard.",
                                InsertMethod::Type => "Types key by key. Leaves the clipboard alone.",
                            }),
                            segmented(
                                "insert-method",
                                vec![
                                    (InsertMethod::Auto, "Auto"),
                                    (InsertMethod::Paste, "Paste"),
                                    (InsertMethod::Type, "Type"),
                                ],
                                cfg.insertion.method,
                                p,
                                {
                                    let set = set.clone();
                                    move |v, _, cx| set(cx, Box::new(move |c| c.insertion.method = v))
                                },
                            ),
                            p,
                        )
                        .into_any_element(),
                    ]
                    .into_iter()
                    // Paste shortcut choice only matters on Linux (Ctrl+V / Cmd+V elsewhere).
                    .chain(cfg!(target_os = "linux").then(|| {
                        row(
                            "Paste shortcut",
                            Some("Shift+Insert works in apps and terminals alike."),
                            paste_keys,
                            p,
                        )
                        .into_any_element()
                    }))
                    .chain(vec![
                        row(
                            "Restore clipboard",
                            Some("Put back what you had copied after pasting."),
                            small_switch("restore-clip", cfg.insertion.restore_clipboard, {
                                let set = set.clone();
                                move |v, _, cx| {
                                    let v = *v;
                                    set(cx, Box::new(move |c| c.insertion.restore_clipboard = v))
                                }
                            }),
                            p,
                        )
                        .into_any_element(),
                        row(
                            "Fit to the surrounding text",
                            Some(SMART_SPACING_HINT),
                            small_switch("smart-spacing", cfg.insertion.smart_spacing, {
                                let set = set.clone();
                                move |v, _, cx| {
                                    let v = *v;
                                    set(cx, Box::new(move |c| c.insertion.smart_spacing = v))
                                }
                            }),
                            p,
                        )
                        .into_any_element(),
                        row(
                            "Add a space after",
                            Some(if cfg.insertion.smart_spacing {
                                "At the end of a line, so back-to-back dictations read as one sentence."
                            } else {
                                "So back-to-back dictations read as one sentence flow."
                            }),
                            small_switch("trailing", cfg.insertion.trailing_space, {
                                let set = set.clone();
                                move |v, _, cx| {
                                    let v = *v;
                                    set(cx, Box::new(move |c| c.insertion.trailing_space = v))
                                }
                            }),
                            p,
                        )
                        .into_any_element(),
                        row(
                            "Split long pastes for Claude Code and Codex",
                            Some("In a terminal running either, long text is pasted in pieces so it shows as text instead of “[Pasted text]”."),
                            small_switch("split-agents", cfg.insertion.split_for_agents, {
                                let set = set.clone();
                                move |v, _, cx| {
                                    let v = *v;
                                    set(cx, Box::new(move |c| c.insertion.split_for_agents = v))
                                }
                            }),
                            p,
                        )
                        .into_any_element(),
                    ])
                    .collect(),
                )),
            )
            .child(
                section("Appearance", p).child(rows(
                    p,
                    vec![
                        row(
                            "Recording indicator",
                            Some("A small pill at the bottom of the screen while you talk."),
                            segmented(
                                "overlay",
                                vec![(OverlayMode::WhileActive, "Show"), (OverlayMode::Never, "Hide")],
                                if cfg.ui.overlay == OverlayMode::Never { OverlayMode::Never } else { OverlayMode::WhileActive },
                                p,
                                {
                                    let set = set.clone();
                                    move |v, _, cx| set(cx, Box::new(move |c| c.ui.overlay = v))
                                },
                            ),
                            p,
                        )
                        .into_any_element(),
                        row(
                            "Theme",
                            None,
                            segmented(
                                "theme",
                                vec![(ThemeMode::System, "System"), (ThemeMode::Light, "Light"), (ThemeMode::Dark, "Dark")],
                                cfg.ui.theme,
                                p,
                                {
                                    let set = set.clone();
                                    move |v, _, cx| set(cx, Box::new(move |c| c.ui.theme = v))
                                },
                            ),
                            p,
                        )
                        .into_any_element(),
                        row(
                            "Start at login",
                            Some("Bars lives in the system tray."),
                            small_switch("autostart", cfg.ui.launch_at_login, {
                                let set = set.clone();
                                move |v, _, cx| {
                                    let v = *v;
                                    set(cx, Box::new(move |c| c.ui.launch_at_login = v))
                                }
                            }),
                            p,
                        )
                        .into_any_element(),
                    ],
                )),
            )
            .child(
                section("Updates", p).child(rows(
                    p,
                    vec![
                        row(
                            "Check for updates automatically",
                            Some("Once a day, Bars fetches a small file from GitHub to see whether a newer version is out. Nothing else is sent, and nothing is installed until you choose to update."),
                            small_switch("check-updates", cfg.ui.check_updates, {
                                let set = set.clone();
                                move |v, _, cx| {
                                    let v = *v;
                                    set(cx, Box::new(move |c| c.ui.check_updates = v))
                                }
                            }),
                            p,
                        )
                        .into_any_element(),
                        row(&version, Some(&update_status), update_button, p).into_any_element(),
                    ],
                )),
            )
    }
}

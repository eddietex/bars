use gpui_kit::assets::IconName as I;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use bars_core::config::{InsertMethod, OverlayMode, PasteKeys, ThemeMode};
use bars_core::controller::Command;
use bars_core::hotkey::{Activation, Mods};

use super::{Page, SettingsView};

#[cfg(target_os = "macos")]
const MODIFIER_ONLY_HINT: &str = "Each one works in every app. Modifier-only shortcuts like Fn or Ctrl + Cmd never type anything, and a middle or side mouse button works too.";
#[cfg(target_os = "windows")]
const MODIFIER_ONLY_HINT: &str = "Each one works in every app. Modifier-only shortcuts like Ctrl + Win never type anything, and a middle or side mouse button works too.";
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
const MODIFIER_ONLY_HINT: &str = "Each one works in every app. Modifier-only shortcuts like Ctrl + Super never type anything, and a middle or side mouse button works too.";

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
        let capture_slot = st.capture_slot;
        let capture_error = st.capture_error.clone();
        // The Globe key also opens the emoji picker/dictation unless told not to.
        let globe_hint = cfg!(target_os = "macos") && cfg.hotkey.chords.iter().any(|c| c.mods.contains(Mods::FN));
        // Linux reads the mouse without holding it: the button also reaches the app.
        let mouse_hint = cfg!(target_os = "linux") && cfg.hotkey.chords.iter().any(|c| c.mouse_button().is_some());
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
        let shortcuts = st.shortcuts();
        let state = self.state.clone();

        let set = {
            let state = state.clone();
            move |cx: &mut App, f: Box<dyn FnOnce(&mut bars_core::config::Config)>| {
                state.update(cx, |s, cx| s.edit_config(cx, f));
            }
        };

        // --- Shortcuts ---------------------------------------------------
        // Without a running shortcut listener no keys would ever arrive:
        // recording opens Setup instead, to show what to allow.
        let view = cx.entity().downgrade();
        let record = |slot: Option<usize>| {
            let (state, view) = (state.clone(), view.clone());
            move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                if !hotkeys_ok {
                    let _ = view.update(cx, |v, cx| v.go(Page::Setup, cx));
                    return;
                }
                state.update(cx, |s, cx| {
                    if s.desktop_shortcut {
                        s.send(Command::ConfigureShortcut);
                    } else {
                        s.start_capture(slot, cx)
                    }
                })
            }
        };
        let listening = || {
            h_row()
                .h(px(34.))
                .px(px(14.))
                .rounded(px(8.))
                .border_1()
                .border_color(p.accent)
                .bg(p.accent_soft)
                .text_size(px(13.))
                .text_color(p.accent_text)
                .child("Press keys or a mouse button…")
        };
        let cancel = |id: &'static str| {
            let state = state.clone();
            button(id, "Cancel", ButtonKind::Ghost, p).on_click(move |_, _, cx| state.update(cx, |s, cx| s.stop_capture(cx)))
        };
        let removable = !desktop_shortcut && shortcuts.len() > 1;
        let mut shortcut_list = v_col().items_end().gap(px(10.));
        for (i, parts) in shortcuts.into_iter().enumerate() {
            let recording_this = capturing && capture_slot == Some(i);
            shortcut_list = shortcut_list.child(if recording_this {
                h_row().gap(px(10.)).child(listening()).child(cancel("cancel-capture")).into_any_element()
            } else {
                h_row()
                    .gap(px(10.))
                    .child(keycaps(parts, p, true))
                    .child(
                        icon_label_button(("change-shortcut", i), I::Pencil, "Change", ButtonKind::Accent, p)
                            .when(!hotkeys_ok, |el| el.opacity(0.5))
                            .on_click(record(Some(i))),
                    )
                    .when(removable, |el| {
                        let state = state.clone();
                        el.child(icon_button(("remove-shortcut", i), I::Trash, p.text_muted, p).on_click(
                            move |_, _, cx| state.update(cx, |s, cx| s.remove_shortcut(i, cx)),
                        ))
                    })
                    .into_any_element()
            });
        }
        if capturing && capture_slot.is_none() {
            shortcut_list = shortcut_list.child(h_row().gap(px(10.)).child(listening()).child(cancel("cancel-add")));
        } else if !desktop_shortcut {
            shortcut_list = shortcut_list.child(
                icon_label_button("add-shortcut", I::Plus, "Add shortcut", ButtonKind::Ghost, p)
                    .when(!hotkeys_ok, |el| el.opacity(0.5))
                    .on_click(record(None)),
            );
        }

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
        let notices_button = button("notices", "View", ButtonKind::Secondary, p).on_click({
            let state = state.clone();
            move |_, _, cx| state.read(cx).send(Command::OpenUrl(third_party_notices()))
        });

        v_col()
            .child(page_header("Shortcut & input", Some("How you start dictating and where the words go."), p))
            .child(
                section("Shortcut", p).child(rows(
                    p,
                    vec![
                        v_col()
                            .child(row(
                                "Dictation shortcuts",
                                Some(if desktop_shortcut {
                                    "Managed by your desktop's shortcut settings. Allow direct keyboard access in Setup to use modifier-only shortcuts, mouse buttons and more than one shortcut."
                                } else {
                                    MODIFIER_ONLY_HINT
                                }),
                                shortcut_list,
                                p,
                            ))
                            .when_some(capture_error, |el, e| {
                                el.child(div().px(px(18.)).pb(px(12.)).text_size(px(12.5)).text_color(p.danger).child(e))
                            })
                            .when(mouse_hint, |el| {
                                el.child(
                                    div()
                                        .px(px(18.))
                                        .pb(px(12.))
                                        .text_size(px(12.5))
                                        .text_color(p.text_muted)
                                        .child("On Linux the mouse button also reaches the app you're in, so pick one it doesn't use (a side button may also go back in a browser)."),
                                )
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
                        row(
                            "Mute other audio",
                            Some("Silences music, videos and other apps while you dictate, and turns them back on after."),
                            small_switch("mute-others", cfg.audio.mute_others, {
                                let set = set.clone();
                                move |v, _, cx| {
                                    let v = *v;
                                    set(cx, Box::new(move |c| c.audio.mute_others = v))
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
                            Some("When it starts and every 24 hours while it runs, Bars fetches a small file from GitHub to see whether a newer version is out. Nothing else is sent, and nothing is installed until you choose to update."),
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
                        row(
                            "Open-source licenses",
                            Some("Bars is MIT-licensed, and built on open-source software by many others."),
                            notices_button,
                            p,
                        )
                        .into_any_element(),
                    ],
                )),
            )
    }
}

/// The licenses of the software Bars is built from: the copy installed with
/// it, or else the one published with this version.
fn third_party_notices() -> String {
    const NAME: &str = "THIRD_PARTY_NOTICES.txt";
    let installed = std::env::current_exe().ok().and_then(|exe| {
        let dir = exe.parent()?.to_path_buf();
        // Next to the program (Windows), in the app's resources (macOS), in
        // the tarball's top folder, or with the Flatpak's licenses.
        [
            dir.join(NAME),
            dir.join("../Resources").join(NAME),
            dir.join("..").join(NAME),
            dir.join("../share/licenses/xyz.hink.Bars").join(NAME),
        ]
        .into_iter()
        .find(|p| p.is_file())
    });
    match installed {
        Some(path) => path.display().to_string(),
        None => format!("https://github.com/hink/bars/blob/v{}/{NAME}", env!("CARGO_PKG_VERSION")),
    }
}

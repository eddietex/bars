use gpui_kit::assets::IconName as I;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use bars_core::config::{InsertMethod, OverlayMode, PasteKeys, ThemeMode};
use bars_core::hotkey::Activation;

use super::SettingsView;

#[cfg(target_os = "macos")]
const MODIFIER_ONLY_HINT: &str = "Works in every app. Modifier-only shortcuts like Ctrl + Cmd never type anything.";
#[cfg(target_os = "windows")]
const MODIFIER_ONLY_HINT: &str = "Works in every app. Modifier-only shortcuts like Ctrl + Win never type anything.";
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
const MODIFIER_ONLY_HINT: &str = "Works in every app. Modifier-only shortcuts like Ctrl + Super never type anything.";
use crate::theme::{Palette, PaletteExt};
use crate::widgets::*;

impl SettingsView {
    pub fn render_general(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = cx.palette();
        let st = self.state.read(cx);
        let cfg = st.config.clone();
        let capturing = st.capturing;
        let capture_error = st.capture_error.clone();
        let hotkeys_ok = st.hotkeys_active;
        let desktop_shortcut = st.desktop_shortcut;
        let shortcut_parts = st.shortcut_parts();
        let devices = st.devices.clone();
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
                            move |_, _, cx| {
                                state.update(cx, |s, cx| {
                                    if s.desktop_shortcut {
                                        s.send(bars_core::controller::Command::ConfigureShortcut);
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
        let mic_options: Vec<(Option<String>, String)> = std::iter::once((None, "System default".to_string()))
            .chain(devices.iter().map(|d| (Some(d.id.clone()), d.name.clone())))
            .collect();
        let mic_picker = self.picker(
            "mic",
            mic_options,
            cfg.audio.device.clone(),
            p,
            {
                let set = set.clone();
                move |v: Option<String>, cx: &mut App| set(cx, Box::new(move |c| c.audio.device = v))
            },
            cx,
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
                            "Add a space after",
                            Some("So back-to-back dictations read as one sentence flow."),
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
    }

    /// A compact dropdown built from a popover menu of options.
    pub fn picker<T: Clone + PartialEq + 'static>(
        &self,
        id: &'static str,
        options: Vec<(T, String)>,
        selected: T,
        p: Palette,
        on_pick: impl Fn(T, &mut App) + 'static,
        _cx: &mut Context<Self>,
    ) -> impl IntoElement {
        use gpui_kit::component::popover::Popover;
        let label = options
            .iter()
            .find(|(v, _)| *v == selected)
            .map(|(_, l)| l.clone())
            .unwrap_or_else(|| "Unavailable".into());
        let on_pick = std::rc::Rc::new(on_pick);
        Popover::new(id)
            .trigger(PickerTrigger { id, label: label.into(), open: false, p })
            .content(move |_, _window, cx| {
                let p = cx.palette();
                let popover = cx.entity();
                let on_pick = on_pick.clone();
                let selected = selected.clone();
                v_col()
                    .w(px(300.))
                    .p(px(4.))
                    .children(options.iter().cloned().enumerate().map(move |(i, (value, label))| {
                        let active = value == selected;
                        let on_pick = on_pick.clone();
                        let popover = popover.clone();
                        h_row()
                            .id(ElementId::NamedInteger(id.into(), i as u64))
                            .h(px(30.))
                            .px(px(10.))
                            .gap(px(8.))
                            .rounded(px(6.))
                            .cursor_pointer()
                            .text_size(px(13.))
                            .hover(move |s| s.bg(p.hover))
                            .on_click(move |_, window, cx| {
                                on_pick(value.clone(), cx);
                                popover.update(cx, |s, cx| s.dismiss(window, cx));
                            })
                            .child(div().flex_1().min_w_0().truncate().child(label))
                            .when(active, |el| el.child(icon(I::Check, p.accent_text, 14.)))
                    }))
            })
    }
}

#[derive(IntoElement)]
struct PickerTrigger {
    id: &'static str,
    label: SharedString,
    open: bool,
    p: Palette,
}

impl gpui_kit::base::Selectable for PickerTrigger {
    fn selected(mut self, selected: bool) -> Self {
        self.open = selected;
        self
    }

    fn is_selected(&self) -> bool {
        self.open
    }
}

impl RenderOnce for PickerTrigger {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let p = self.p;
        h_row()
            .id(SharedString::from(format!("{}-trigger", self.id)))
            .h(px(32.))
            .w(px(260.))
            .px(px(12.))
            .gap(px(8.))
            .justify_between()
            .rounded(px(8.))
            .bg(p.surface_2)
            .border_1()
            .border_color(if self.open { p.accent } else { p.border })
            .cursor_pointer()
            .hover(move |s| s.border_color(p.border_strong))
            .child(div().flex_1().min_w_0().truncate().text_size(px(13.)).child(self.label))
            .child(icon(I::ChevronsUpDown, p.text_faint, 14.))
    }
}

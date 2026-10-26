use gpui_kit::assets::IconName as I;
use gpui_kit::component::input::Input;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use bars_core::controller::Command;
use bars_core::models;
use bars_core::platform::requirements::{FixAction, ReqId, Requirement};

use super::{Page, SettingsView};
use crate::theme::{Palette, PaletteExt};
use crate::widgets::*;

#[derive(Clone, Copy, PartialEq)]
enum StepState {
    Done,
    Todo,
    Busy,
}

/// What the Status rows show.
struct StatusFacts {
    hotkeys_ok: bool,
    keyboard_ok: bool,
    clipboard_ok: bool,
    desktop_shortcut: bool,
    restore_clipboard: bool,
}

impl SettingsView {
    pub fn render_setup(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = cx.palette();
        let st = self.state.read(cx);
        let model = st.active_model().unwrap_or(&models::CATALOG[0]);
        let model_installed = st.installed.contains(model.id);
        let downloading = st.downloads.get(model.id).copied();
        let perms = st.permissions.clone();
        let hotkeys_ok = st.hotkeys_active;
        let keyboard_ok = perms.keyboard;
        let chord_parts = st.shortcut_parts();
        let desktop_shortcut = st.desktop_shortcut;
        let restore_clipboard = st.config.insertion.restore_clipboard;

        let model_detail = format!("{} · {} · runs fully offline", model.name, fmt_bytes(model.size_bytes()));
        let dictated = st.stats.dictations > 0;
        let setup_complete = st.setup_complete();
        let model_step = step(
            1,
            "Download the speech model",
            &model_detail,
            if model_installed {
                StepState::Done
            } else if downloading.is_some() {
                StepState::Busy
            } else {
                StepState::Todo
            },
            p,
            if model_installed {
                None
            } else if let Some((done, total)) = downloading {
                Some(
                    v_col()
                        .gap(px(6.))
                        .w(px(220.))
                        .child(progress_bar(if total > 0 { done as f32 / total as f32 } else { 0.0 }, p))
                        .child(
                            div()
                                .text_size(px(12.))
                                .text_color(p.text_muted)
                                .child(format!("{} of {}", fmt_bytes(done), fmt_bytes(total))),
                        )
                        .into_any_element(),
                )
            } else {
                let id = model.id;
                Some(
                    button("setup-dl", "Download", ButtonKind::Primary, p)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.state.read(cx).send(Command::DownloadModel(id.to_string()))
                        }))
                        .into_any_element(),
                )
            },
        )
        .into_any_element();

        // One step per thing this platform needs granted, between the model
        // and the first dictation.
        let mut steps = vec![model_step];
        for (i, req) in perms.items.iter().enumerate() {
            let n = steps.len() as u8 + 1;
            steps.push(if req.id == ReqId::KeyboardAccess {
                self.keyboard_step(n, &perms, hotkeys_ok, desktop_shortcut, p, cx)
            } else {
                requirement_step(n, i, req, p, cx)
            });
        }
        let try_n = steps.len() as u8 + 1;
        steps.push(
            step(
                try_n,
                "Try it",
                "Click the field below, hold your shortcut, say something, then let go.",
                if dictated { StepState::Done } else { StepState::Todo },
                p,
                Some(keycaps(chord_parts, p, false).into_any_element()),
            )
            .into_any_element(),
        );
        let subtitle = format!("{} quick steps and you're dictating.", count_word(steps.len()));

        let mut card_el = card(p);
        for (i, s) in steps.into_iter().enumerate() {
            if i > 0 {
                card_el = card_el.child(div().px(px(18.)).child(divider(p)));
            }
            card_el = card_el.child(s);
        }
        let facts = StatusFacts {
            hotkeys_ok,
            keyboard_ok,
            clipboard_ok: perms.clipboard,
            desktop_shortcut,
            restore_clipboard,
        };

        v_col()
            .child(
                h_row()
                    .justify_between()
                    .items_start()
                    .child(page_header("Setup", Some(subtitle.as_str()), p))
                    .child(
                        button("assistant", "Open setup assistant", ButtonKind::Secondary, p)
                            .on_click(|_, _, cx| crate::shell::send(cx, crate::shell::ShellCommand::ShowOnboarding)),
                    ),
            )
            .child(card_el.child(div().px(px(18.)).pb(px(18.)).child(Input::new(&self.practice))))
            .child(section("Status", p).mt(px(24.)).child(rows(p, status_rows(&facts, p, cx))))
            .when(setup_complete, |el| {
                el.child(
                    h_row().justify_end().child(
                        button("done", "Done", ButtonKind::Primary, p)
                            .on_click(cx.listener(|this, _, _, cx| this.go(Page::Home, cx))),
                    ),
                )
            })
    }

    /// Linux: reading keyboards and typing, directly (udev rule) or through
    /// the desktop's portals.
    fn keyboard_step(
        &self,
        n: u8,
        perms: &bars_core::platform::Permissions,
        hotkeys_ok: bool,
        desktop_shortcut: bool,
        p: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let access_ok = hotkeys_ok && perms.keyboard && !desktop_shortcut;
        let access_detail = if access_ok {
            "Bars can see your shortcut and type into any app.".to_string()
        } else if hotkeys_ok {
            "Working through your desktop's shortcut and remote-control portals. Allow direct access for modifier-only shortcuts like Ctrl + Super and silent typing.".to_string()
        } else {
            "Lets Bars notice your shortcut and type the result, like a keyboard. Granted only to you, while you're logged in.".to_string()
        };
        let access_action: Option<AnyElement> = if access_ok {
            None
        } else if perms.can_fix_automatically {
            Some(
                h_row()
                    .gap(px(6.))
                    .child(
                        button("recheck", "Check again", ButtonKind::Ghost, p).on_click(
                            cx.listener(|this, _, _, cx| this.state.read(cx).send(Command::RefreshPermissions)),
                        ),
                    )
                    .child(
                        button("grant", "Allow access", ButtonKind::Primary, p).on_click(
                            cx.listener(|this, _, _, cx| this.state.read(cx).send(Command::FixPermissions(None))),
                        ),
                    )
                    .into_any_element(),
            )
        } else {
            Some(
                button("recheck", "Check again", ButtonKind::Secondary, p)
                    .on_click(cx.listener(|this, _, _, cx| this.state.read(cx).send(Command::RefreshPermissions)))
                    .into_any_element(),
            )
        };
        v_col()
            .child(step(
                n,
                "Allow keyboard access",
                &access_detail,
                if access_ok { StepState::Done } else { StepState::Todo },
                p,
                access_action,
            ))
            .when_some(
                (!access_ok && !perms.can_fix_automatically)
                    .then(|| perms.fix_command.clone())
                    .flatten()
                    .map(|cmd| command_box(cmd, p, cx)),
                |el, b| el.child(b),
            )
            .into_any_element()
    }
}

/// The microphone, and on macOS Accessibility, worded and fixed the way
/// `requirements` says for this platform.
fn requirement_step(n: u8, i: usize, req: &Requirement, p: Palette, cx: &mut Context<SettingsView>) -> AnyElement {
    let title = match req.id {
        ReqId::Microphone if cfg!(target_os = "linux") => "Connect a microphone".to_string(),
        ReqId::Microphone => "Allow the microphone".to_string(),
        _ => format!("Allow {}", req.title),
    };
    let no_device = req.id == ReqId::Microphone && req.granted_note.is_none();
    let detail = if req.ok() {
        req.granted_note.clone().unwrap_or_else(|| req.why.clone())
    } else if no_device {
        "No microphone found. Plug one in, then press Check again.".to_string()
    } else {
        req.why.clone()
    };
    let action = (!req.ok()).then(|| {
        let id = req.id;
        let recheck = |kind| {
            button(("req-recheck", i), "Check again", kind, p)
                .on_click(cx.listener(|this, _, _, cx| this.state.read(cx).send(Command::RefreshPermissions)))
        };
        let fix: Option<AnyElement> = if no_device {
            None
        } else if req.actions.contains(&FixAction::Elevate) {
            Some(
                button(("req-allow", i), "Allow access", ButtonKind::Primary, p)
                    .on_click(cx.listener(move |this, _, _, cx| this.state.read(cx).send(Command::FixPermissions(Some(id)))))
                    .into_any_element(),
            )
        } else {
            req.actions.iter().find_map(|a| match a {
                FixAction::OpenSettings { label, url } => {
                    let url = url.clone();
                    Some(
                        button(("req-settings", i), label.clone(), ButtonKind::Primary, p)
                            .on_click(cx.listener(move |this, _, _, cx| this.state.read(cx).send(Command::OpenUrl(url.clone()))))
                            .into_any_element(),
                    )
                }
                _ => None,
            })
        };
        match fix {
            Some(fix) => h_row().gap(px(6.)).child(recheck(ButtonKind::Ghost)).child(fix).into_any_element(),
            None => recheck(ButtonKind::Secondary).into_any_element(),
        }
    });
    step(n, &title, &detail, if req.ok() { StepState::Done } else { StepState::Todo }, p, action).into_any_element()
}

fn count_word(n: usize) -> String {
    match n {
        2 => "Two".into(),
        3 => "Three".into(),
        4 => "Four".into(),
        5 => "Five".into(),
        n => n.to_string(),
    }
}

#[cfg(target_os = "macos")]
fn status_rows(f: &StatusFacts, p: Palette, cx: &mut Context<SettingsView>) -> Vec<AnyElement> {
    use bars_core::platform::mac::permissions;
    let pane = permissions::accessibility_pane();
    let _ = (f.clipboard_ok, f.desktop_shortcut);
    let restore_ok = bars_core::platform::clipboard_restore_allowed();
    vec![
        check_row(
            "Global shortcut",
            f.hotkeys_ok,
            &if !f.hotkeys_ok {
                format!("Not running yet: allow {pane} above")
            } else if permissions::full_listener() {
                "Sees your shortcut in every app".to_string()
            } else {
                "Listen-only: Esc and your shortcut's key also reach the app".to_string()
            },
            p,
            None,
        ),
        check_row(
            "Typing",
            f.keyboard_ok,
            &if f.keyboard_ok {
                "Pastes with Cmd+V into the app you're using".to_string()
            } else {
                format!("Needs {pane}")
            },
            p,
            None,
        ),
        check_row(
            "Clipboard",
            !f.restore_clipboard || restore_ok,
            if !f.restore_clipboard {
                "Restore clipboard is off, so your dictation stays on the clipboard"
            } else if restore_ok {
                "Puts back what you had copied after each paste"
            } else {
                "macOS asks before Bars reads the clipboard, so your dictation stays on it"
            },
            p,
            (f.restore_clipboard && !restore_ok).then(|| {
                button("setup-allow-clipboard", "Allow…", ButtonKind::Accent, p)
                    .on_click(cx.listener(|this, _, _, cx| this.state.read(cx).send(Command::AllowClipboardReads)))
                    .into_any_element()
            }),
        ),
    ]
}

#[cfg(target_os = "windows")]
fn status_rows(f: &StatusFacts, p: Palette, _cx: &mut Context<SettingsView>) -> Vec<AnyElement> {
    let _ = (f.keyboard_ok, f.desktop_shortcut);
    vec![
        check_row(
            "Global shortcut",
            f.hotkeys_ok,
            if f.hotkeys_ok { "Sees your shortcut in every app" } else { "The shortcut listener couldn't start" },
            p,
            None,
        ),
        check_row("Typing", true, "Types or pastes into the app you're using", p, None),
        check_row(
            "Clipboard",
            f.clipboard_ok,
            if !f.clipboard_ok {
                "The clipboard isn't available, so Bars types instead"
            } else if f.restore_clipboard {
                "Pastes, then puts back what you had copied"
            } else {
                "Restore clipboard is off, so your dictation stays on the clipboard"
            },
            p,
            None,
        ),
    ]
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn status_rows(f: &StatusFacts, p: Palette, _cx: &mut Context<SettingsView>) -> Vec<AnyElement> {
    let _ = f.restore_clipboard;
    vec![
        check_row(
            "Global shortcut",
            f.hotkeys_ok,
            if !f.hotkeys_ok {
                "Not available yet"
            } else if f.desktop_shortcut {
                "Via your desktop's shortcut settings"
            } else {
                "Reads keyboards to notice your shortcut"
            },
            p,
            None,
        ),
        check_row(
            "Typing",
            true,
            if f.keyboard_ok {
                "Virtual keyboard: types or pastes into the focused app"
            } else {
                "Via the desktop remote-control portal (asks once)"
            },
            p,
            None,
        ),
        check_row(
            "Clipboard",
            f.clipboard_ok,
            if f.clipboard_ok {
                "Fast paste with clipboard restore"
            } else {
                "Not available on this desktop — Bars will type instead"
            },
            p,
            None,
        ),
    ]
}
fn step(
    n: u8,
    title: &str,
    detail: &str,
    state: StepState,
    p: Palette,
    action: Option<AnyElement>,
) -> impl IntoElement {
    h_row()
        .gap(px(16.))
        .px(px(18.))
        .py(px(16.))
        .child(
            div()
                .size(px(28.))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .rounded_full()
                .when(state == StepState::Done, |el| el.bg(p.success))
                .when(state != StepState::Done, |el| el.border_1().border_color(p.border_strong))
                .child(if state == StepState::Done {
                    icon(I::Check, rgb(0xffffff).into(), 15.).into_any_element()
                } else {
                    div()
                        .text_size(px(12.5))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(if state == StepState::Busy { p.accent_text } else { p.text_muted })
                        .child(n.to_string())
                        .into_any_element()
                }),
        )
        .child(
            v_col()
                .flex_1()
                .min_w_0()
                .gap(px(3.))
                .child(div().text_size(px(14.)).font_weight(FontWeight::MEDIUM).child(title.to_string()))
                .child(
                    div()
                        .text_size(px(12.5))
                        .line_height(px(18.))
                        .text_color(p.text_muted)
                        .child(detail.to_string()),
                ),
        )
        .when_some(action, |el, a| el.child(div().flex_none().child(a)))
}

fn check_row(title: &str, ok: bool, detail: &str, p: Palette, action: Option<AnyElement>) -> AnyElement {
    h_row()
        .gap(px(12.))
        .px(px(18.))
        .py(px(12.))
        .child(icon(
            if ok { I::CircleCheck } else { I::CircleAlert },
            if ok { p.success } else { p.warning },
            16.,
        ))
        .child(div().w(px(150.)).text_size(px(13.5)).font_weight(FontWeight::MEDIUM).child(title.to_string()))
        .child(div().flex_1().text_size(px(12.5)).text_color(p.text_muted).child(detail.to_string()))
        .when_some(action, |el, a| el.child(div().flex_none().child(a)))
        .into_any_element()
}
fn command_box(cmd: String, p: Palette, cx: &mut Context<SettingsView>) -> impl IntoElement {
    let copy = cmd.clone();
    v_col()
        .mx(px(18.))
        .mb(px(16.))
        .ml(px(62.))
        .gap(px(8.))
        .child(
            div()
                .text_size(px(12.5))
                .text_color(p.text_muted)
                .child("Run this once in a terminal (it installs a small udev rule), then press Check again:"),
        )
        .child(
            h_row()
                .gap(px(8.))
                .p(px(10.))
                .rounded(px(8.))
                .bg(p.surface_2)
                .border_1()
                .border_color(p.border)
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .font_family("monospace")
                        .text_size(px(11.5))
                        .text_color(p.text)
                        .child("sudo sh -c '… install 70-bars.rules …'"),
                )
                .child(
                    button("copy-cmd", "Copy command", ButtonKind::Secondary, p).on_click(cx.listener(
                        move |this, _, _, cx| this.state.read(cx).send(Command::CopyText(copy.clone())),
                    )),
                ),
        )
}

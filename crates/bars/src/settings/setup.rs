use gpui_kit::assets::IconName as I;
use gpui_kit::component::input::Input;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use bars_core::controller::Command;
use bars_core::models;

use super::{Page, SettingsView};
use crate::theme::{Palette, PaletteExt};
use crate::widgets::*;

#[derive(Clone, Copy, PartialEq)]
enum StepState {
    Done,
    Todo,
    Busy,
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
        );

        let access_ok = hotkeys_ok && keyboard_ok && !desktop_shortcut;
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
                            cx.listener(|this, _, _, cx| this.state.read(cx).send(Command::FixPermissions)),
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
        let access_step = v_col()
            .child(step(
                2,
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
            );

        let try_step = step(
            3,
            "Try it",
            "Click the field below, hold your shortcut, say something, then let go.",
            if dictated { StepState::Done } else { StepState::Todo },
            p,
            Some(keycaps(chord_parts, p, false).into_any_element()),
        );

        v_col()
            .child(
                h_row()
                    .justify_between()
                    .items_start()
                    .child(page_header("Setup", Some("Three quick steps and you're dictating."), p))
                    .child(
                        button("assistant", "Open setup assistant", ButtonKind::Secondary, p)
                            .on_click(|_, _, cx| crate::shell::send(cx, crate::shell::ShellCommand::ShowOnboarding)),
                    ),
            )
            .child(
                card(p)
                    .child(model_step)
                    .child(div().px(px(18.)).child(divider(p)))
                    .child(access_step)
                    .child(div().px(px(18.)).child(divider(p)))
                    .child(try_step)
                    .child(div().px(px(18.)).pb(px(18.)).child(Input::new(&self.practice))),
            )
            .child(
                section("Status", p).mt(px(24.)).child(rows(
                    p,
                    vec![
                        check_row(
                            "Global shortcut",
                            hotkeys_ok,
                            if !hotkeys_ok {
                                "Not available yet"
                            } else if desktop_shortcut {
                                "Via your desktop's shortcut settings"
                            } else {
                                "Reads keyboards to notice your shortcut"
                            },
                            p,
                        ),
                        check_row(
                            "Typing",
                            true,
                            if keyboard_ok {
                                "Virtual keyboard: types or pastes into the focused app"
                            } else {
                                "Via the desktop remote-control portal (asks once)"
                            },
                            p,
                        ),
                        check_row(
                            "Clipboard",
                            perms.clipboard,
                            if perms.clipboard {
                                "Fast paste with clipboard restore"
                            } else {
                                "Not available on this desktop — Bars will type instead"
                            },
                            p,
                        ),
                    ],
                )),
            )
            .when(setup_complete, |el| {
                el.child(
                    h_row().justify_end().child(
                        button("done", "Done", ButtonKind::Primary, p)
                            .on_click(cx.listener(|this, _, _, cx| this.go(Page::Home, cx))),
                    ),
                )
            })
    }
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

fn check_row(title: &str, ok: bool, detail: &str, p: Palette) -> AnyElement {
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


use gpui_kit::assets::IconName as I;
use gpui_kit::component::input::{Textarea, TextareaState};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use bars_core::controller::{Command, Phase};
use bars_core::history::Entry;
use bars_core::hotkey::Activation;

use super::{Page, SettingsView};
use crate::theme::{Palette, PaletteExt};
use crate::widgets::*;

impl SettingsView {
    pub fn render_home(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = cx.palette();
        let st = self.state.read(cx);
        let chord_parts = st.shortcut_parts();
        let activation = st.config.hotkey.activation;
        let stats = st.stats.clone();
        let recent: Vec<Entry> = st.history.iter().rev().take(6).cloned().collect();
        let phase = st.phase;
        let needs_setup = !st.setup_complete();

        let how = match activation {
            Activation::HoldOrDoubleTap => "Hold to talk, release to insert. Double-tap to go hands-free.",
            Activation::Hold => "Hold to talk, release to insert.",
            Activation::Toggle => "Press once to start, again to insert.",
        };

        v_col()
            .when(needs_setup, |el| el.child(self.setup_banner(p, cx)))
            .child(
                // Hero: the one thing to remember.
                v_col()
                    .items_center()
                    .gap(px(14.))
                    .py(px(34.))
                    .px(px(24.))
                    .mb(px(20.))
                    .rounded(px(16.))
                    .bg(p.surface)
                    .border_1()
                    .border_color(p.border)
                    .child(match phase {
                        Phase::Recording { .. } => badge("Listening", p.recording).into_any_element(),
                        Phase::Transcribing => badge("Transcribing", p.accent).into_any_element(),
                        Phase::Idle => div()
                            .text_size(px(12.5))
                            .text_color(p.text_muted)
                            .child("Dictate anywhere")
                            .into_any_element(),
                    })
                    .child(keycaps(chord_parts, p, true))
                    .child(div().text_size(px(13.5)).text_color(p.text_muted).child(how)),
            )
            .child(
                h_row()
                    .gap(px(12.))
                    .mb(px(28.))
                    .child(stat("Words dictated", fmt_number(stats.words), Some(format!("{} dictations", stats.dictations)), p))
                    .child(stat(
                        "Speaking pace",
                        if stats.wpm > 0.0 { format!("{:.0} wpm", stats.wpm) } else { "—".into() },
                        Some("vs ~40 wpm typing".into()),
                        p,
                    ))
                    .child(stat(
                        "Time saved",
                        fmt_minutes(stats.minutes_saved),
                        Some(if stats.streak_days > 0 {
                            format!("{}-day streak", stats.streak_days)
                        } else {
                            "start a streak today".into()
                        }),
                        p,
                    ))
                    .child(stat(
                        "Avg. latency",
                        if stats.dictations > 0 { format!("{:.0} ms", stats.avg_latency_ms) } else { "—".into() },
                        Some("on-device".into()),
                        p,
                    )),
            )
            .child(
                h_row()
                    .justify_between()
                    .mb(px(10.))
                    .px(px(4.))
                    .child(
                        div()
                            .text_size(px(12.5))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(p.text_muted)
                            .child("Recent"),
                    )
                    .when(!recent.is_empty(), |el| {
                        el.child(
                            div()
                                .id("see-all")
                                .text_size(px(12.5))
                                .text_color(p.accent_text)
                                .cursor_pointer()
                                .on_click(cx.listener(|this, _, _, cx| this.go(Page::History, cx)))
                                .child("See all"),
                        )
                    }),
            )
            .child(if recent.is_empty() {
                card(p)
                    .p(px(28.))
                    .items_center()
                    .gap(px(6.))
                    .child(icon(I::AudioLines, p.text_faint, 22.))
                    .child(div().text_size(px(13.5)).text_color(p.text_muted).child("Your dictations will show up here."))
                    .into_any_element()
            } else {
                let items = recent.iter().map(|e| self.entry_row(e, p, cx).into_any_element()).collect();
                rows(p, items).into_any_element()
            })
    }

    fn setup_banner(&self, p: Palette, cx: &mut Context<Self>) -> impl IntoElement {
        h_row()
            .gap(px(14.))
            .p(px(16.))
            .mb(px(20.))
            .rounded(px(12.))
            .bg(p.accent_soft)
            .border_1()
            .border_color(p.accent_soft)
            .child(icon(I::Sparkles, p.accent_text, 18.))
            .child(
                v_col()
                    .flex_1()
                    .gap(px(2.))
                    .child(div().text_size(px(14.)).font_weight(FontWeight::SEMIBOLD).child("Finish setting up Bars"))
                    .child(
                        div()
                            .text_size(px(12.5))
                            .text_color(p.text_muted)
                            .child("Download a speech model and allow keyboard access — it takes a minute."),
                    ),
            )
            .child(
                button("go-setup", "Continue", ButtonKind::Primary, p)
                    .on_click(cx.listener(|this, _, _, cx| this.go(Page::Setup, cx))),
            )
    }

    pub fn entry_row(&self, e: &Entry, p: Palette, cx: &mut Context<Self>) -> impl IntoElement {
        let id = e.id;
        let text = e.text.clone();
        let copied = self.copied == Some(id);
        let editor = self.editing.as_ref().filter(|(eid, _)| *eid == id).map(|(_, ed)| ed.clone());
        h_row()
            .id(ElementId::NamedInteger("entry".into(), id))
            .group("entry")
            .gap(px(14.))
            .px(px(18.))
            .py(px(12.))
            .items_start()
            .child(
                div()
                    .w(px(56.))
                    .flex_none()
                    .pt(px(1.))
                    .text_size(px(12.))
                    .text_color(p.text_faint)
                    .child(e.at.format("%H:%M").to_string()),
            )
            .child(match editor {
                // Correcting it here also teaches Bars the fix.
                Some(editor) => v_col()
                    .flex_1()
                    .min_w_0()
                    .gap(px(8.))
                    .child(
                        div()
                            .px(px(10.))
                            .py(px(6.))
                            .rounded(px(8.))
                            .border_1()
                            .border_color(p.accent)
                            .bg(p.surface)
                            .text_size(px(13.5))
                            .child(Textarea::new(&editor)),
                    )
                    .child(
                        h_row()
                            .gap(px(8.))
                            .child(button(ElementId::NamedInteger("save".into(), id), "Save", ButtonKind::Primary, p).on_click(
                                cx.listener(move |this, _, _, cx| {
                                    if let Some((_, ed)) = this.editing.take() {
                                        let text = ed.read(cx).value().to_string();
                                        this.state.read(cx).send(Command::EditEntry { id, text });
                                    }
                                    cx.notify();
                                }),
                            ))
                            .child(
                                button(ElementId::NamedInteger("cancel".into(), id), "Cancel", ButtonKind::Ghost, p)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.editing = None;
                                        cx.notify();
                                    })),
                            ),
                    )
                    .into_any_element(),
                None => div()
                    .flex_1()
                    .min_w_0()
                    .text_size(px(13.5))
                    .line_height(px(20.))
                    .text_color(p.text)
                    .child(e.text.clone())
                    .into_any_element(),
            })
            .child(
                h_row()
                    .flex_none()
                    .gap(px(2.))
                    .invisible()
                    .group_hover("entry", |s| s.visible())
                    .child(
                        icon_button(ElementId::NamedInteger("edit".into(), id), I::Pencil, p.text_muted, p).on_click(
                            cx.listener({
                                let text = text.clone();
                                move |this, _, window, cx| {
                                    let text = text.clone();
                                    let editor = cx.new(|cx| {
                                        TextareaState::new(window, cx).auto_grow(1, 8).default_value(text)
                                    });
                                    editor.update(cx, |ed, cx| ed.focus(window, cx));
                                    this.editing = Some((id, editor));
                                    cx.notify();
                                }
                            }),
                        ),
                    )
                    .child(
                        icon_button(
                            ElementId::NamedInteger("copy".into(), id),
                            if copied { I::Check } else { I::Copy },
                            if copied { p.success } else { p.text_muted },
                            p,
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.state.read(cx).send(Command::CopyText(text.clone()));
                            this.copied = Some(id);
                            cx.notify();
                        })),
                    )
                    .child(
                        icon_button(ElementId::NamedInteger("del".into(), id), I::Trash, p.text_muted, p)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.state.read(cx).send(Command::DeleteEntry(id));
                            })),
                    ),
            )
    }
}

fn fmt_minutes(m: f64) -> String {
    if m <= 0.0 {
        "—".into()
    } else if m < 60.0 {
        format!("{m:.0} min")
    } else {
        format!("{:.1} h", m / 60.0)
    }
}

use gpui_kit::assets::IconName as I;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::*;
use bars_core::config::{Replacement, Snippet};

use super::SettingsView;
use crate::theme::{Palette, PaletteExt};
use crate::widgets::*;

impl SettingsView {
    pub fn render_dictionary(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = cx.palette();
        let st = self.state.read(cx);
        let text_cfg = st.config.text.clone();
        let state = self.state.clone();

        let toggles = rows(
            p,
            vec![
                row(
                    "Remove filler words",
                    Some("Drops “um”, “uh”, “erm” and friends."),
                    small_switch("fillers", text_cfg.remove_fillers, {
                        let state = state.clone();
                        move |v, _, cx| {
                            let v = *v;
                            state.update(cx, |s, cx| s.edit_config(cx, |c| c.text.remove_fillers = v))
                        }
                    }),
                    p,
                )
                .into_any_element(),
                row(
                    "Voice commands",
                    Some("Say “new line” or “new paragraph” to break lines."),
                    small_switch("voice-cmds", text_cfg.voice_commands, {
                        let state = state.clone();
                        move |v, _, cx| {
                            let v = *v;
                            state.update(cx, |s, cx| s.edit_config(cx, |c| c.text.voice_commands = v))
                        }
                    }),
                    p,
                )
                .into_any_element(),
            ],
        );

        let replacements: Vec<AnyElement> = text_cfg
            .dictionary
            .iter()
            .enumerate()
            .map(|(i, r)| {
                pair_row(
                    ElementId::NamedInteger("rep".into(), i as u64),
                    &r.from,
                    &r.to,
                    p,
                    cx.listener(move |this, _, _, cx| {
                        this.state.update(cx, |s, cx| {
                            s.edit_config(cx, |c| {
                                if i < c.text.dictionary.len() {
                                    c.text.dictionary.remove(i);
                                }
                            })
                        })
                    }),
                )
                .into_any_element()
            })
            .collect();

        let snippets: Vec<AnyElement> = text_cfg
            .snippets
            .iter()
            .enumerate()
            .map(|(i, s)| {
                pair_row(
                    ElementId::NamedInteger("snip".into(), i as u64),
                    &s.trigger,
                    &s.expansion,
                    p,
                    cx.listener(move |this, _, _, cx| {
                        this.state.update(cx, |s, cx| {
                            s.edit_config(cx, |c| {
                                if i < c.text.snippets.len() {
                                    c.text.snippets.remove(i);
                                }
                            })
                        })
                    }),
                )
                .into_any_element()
            })
            .collect();

        v_col()
            .child(page_header(
                "Dictionary",
                Some("Teach Bars your words, names and shortcuts."),
                p,
            ))
            .child(section("Clean-up", p).child(toggles))
            .child(
                section("Word replacements", p)
                    .child(adder(
                        &self.dict_from,
                        &self.dict_to,
                        "add-rep",
                        p,
                        cx.listener(|this, _, window, cx| {
                            let (from, to) = (this.dict_from.read(cx).value(), this.dict_to.read(cx).value());
                            if from.trim().is_empty() || to.trim().is_empty() {
                                return;
                            }
                            this.state.update(cx, |s, cx| {
                                s.edit_config(cx, |c| {
                                    c.text.dictionary.push(Replacement {
                                        from: from.trim().to_string(),
                                        to: to.trim().to_string(),
                                    })
                                })
                            });
                            clear(&this.dict_from, window, cx);
                            clear(&this.dict_to, window, cx);
                        }),
                    ))
                    .child(list_or_empty(replacements, "Fix names and jargon the model mishears, e.g. “jason” → JSON.", p)),
            )
            .child(
                section("Snippets", p)
                    .child(adder(
                        &self.snip_trigger,
                        &self.snip_text,
                        "add-snip",
                        p,
                        cx.listener(|this, _, window, cx| {
                            let (trigger, expansion) =
                                (this.snip_trigger.read(cx).value(), this.snip_text.read(cx).value());
                            if trigger.trim().is_empty() || expansion.trim().is_empty() {
                                return;
                            }
                            this.state.update(cx, |s, cx| {
                                s.edit_config(cx, |c| {
                                    c.text.snippets.push(Snippet {
                                        trigger: trigger.trim().to_string(),
                                        expansion: expansion.to_string(),
                                    })
                                })
                            });
                            clear(&this.snip_trigger, window, cx);
                            clear(&this.snip_text, window, cx);
                        }),
                    ))
                    .child(list_or_empty(snippets, "Say a cue like “my calendar link” to insert saved text.", p)),
            )
    }
}

fn clear(input: &Entity<InputState>, window: &mut Window, cx: &mut App) {
    input.update(cx, |s, cx| s.set_value("", window, cx));
}

fn adder(
    left: &Entity<InputState>,
    right: &Entity<InputState>,
    id: &'static str,
    p: Palette,
    on_add: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    h_row()
        .gap(px(8.))
        .child(div().flex_1().child(Input::new(left)))
        .child(icon(I::ArrowRight, p.text_faint, 14.))
        .child(div().flex_1().child(Input::new(right)))
        .child(button(id, "Add", ButtonKind::Secondary, p).on_click(on_add))
}

fn pair_row(
    id: ElementId,
    left: &str,
    right: &str,
    p: Palette,
    on_remove: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    h_row()
        .id(id.clone())
        .group("pair")
        .px(px(18.))
        .py(px(10.))
        .gap(px(12.))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_size(px(13.5))
                .text_color(p.text_muted)
                .child(left.to_string()),
        )
        .child(icon(I::ArrowRight, p.text_faint, 13.))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_size(px(13.5))
                .font_weight(FontWeight::MEDIUM)
                .child(right.replace('\n', " ⏎ ")),
        )
        .child(
            div()
                .invisible()
                .group_hover("pair", |s| s.visible())
                .child(icon_button(SharedString::from(format!("{id}-rm")), I::X, p.text_muted, p).on_click(on_remove)),
        )
}

fn list_or_empty(items: Vec<AnyElement>, empty: &str, p: Palette) -> impl IntoElement {
    if items.is_empty() {
        div()
            .px(px(4.))
            .pt(px(2.))
            .text_size(px(12.5))
            .text_color(p.text_faint)
            .child(empty.to_string())
            .into_any_element()
    } else {
        rows(p, items).mt(px(4.)).into_any_element()
    }
}


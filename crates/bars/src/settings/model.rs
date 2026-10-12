use gpui_kit::assets::IconName as I;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use bars_core::controller::Command;
use bars_core::engine::EngineStatus;
use bars_core::models::{self, ModelInfo};

use super::SettingsView;
use crate::theme::{Palette, PaletteExt};
use crate::widgets::*;

const LANGUAGES: &[(&str, &str)] = &[
    ("en", "English"),
    ("de", "German"),
    ("es", "Spanish"),
    ("fr", "French"),
    ("it", "Italian"),
    ("pt", "Portuguese"),
    ("nl", "Dutch"),
    ("pl", "Polish"),
    ("sv", "Swedish"),
    ("da", "Danish"),
    ("fi", "Finnish"),
    ("cs", "Czech"),
    ("uk", "Ukrainian"),
    ("ru", "Russian"),
];

impl SettingsView {
    pub fn render_model(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = cx.palette();
        let st = self.state.read(cx);
        let active_id = st.config.model.id.clone();
        let multilingual = st.active_model().is_some_and(|m| m.multilingual);
        let language = st.config.model.language.clone();
        let engine = st.engine.clone();

        let cards: Vec<AnyElement> = models::CATALOG
            .iter()
            .map(|m| self.model_card(m, m.id == active_id, &engine, p, cx).into_any_element())
            .collect();

        let lang_options: Vec<(Option<String>, String)> = std::iter::once((None, "Auto-detect".to_string()))
            .chain(LANGUAGES.iter().map(|(c, n)| (Some(c.to_string()), n.to_string())))
            .collect();
        let state = self.state.clone();
        let lang_picker = picker(
            "language",
            lang_options,
            language,
            p,
            move |v, cx| state.update(cx, |s, cx| s.edit_config(cx, |c| c.model.language = v)),
        );

        v_col()
            .child(page_header(
                "Speech model",
                Some("Everything runs on this computer. Audio never leaves it."),
                p,
            ))
            .child(v_col().gap(px(12.)).mb(px(28.)).children(cards))
            .when(multilingual, |el| {
                el.child(section("Language", p).child(rows(
                    p,
                    vec![row(
                        "Spoken language",
                        Some("Auto-detect works well; pin a language if you only speak one."),
                        lang_picker,
                        p,
                    )
                    .into_any_element()],
                )))
            })
    }

    fn model_card(
        &self,
        m: &'static ModelInfo,
        active: bool,
        engine: &EngineStatus,
        p: Palette,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let st = self.state.read(cx);
        let installed = st.installed.contains(m.id);
        let progress = st.downloads.get(m.id).copied();
        let error = st.download_errors.get(m.id).cloned();
        let id = m.id;

        let status: AnyElement = if active {
            match engine {
                EngineStatus::Loading(_) => badge("Loading…", p.accent).into_any_element(),
                EngineStatus::Failed { .. } => badge("Failed to load", p.danger).into_any_element(),
                _ if installed => badge("In use", p.success).into_any_element(),
                _ => badge("Not downloaded", p.warning).into_any_element(),
            }
        } else if installed {
            badge("Downloaded", p.text_muted).into_any_element()
        } else {
            div().into_any_element()
        };

        let actions: AnyElement = if let Some((done, total)) = progress {
            h_row()
                .gap(px(10.))
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(p.text_muted)
                        .child(format!("{} / {}", fmt_bytes(done), fmt_bytes(total))),
                )
                .child(
                    button(SharedString::from(format!("cancel-{id}")), "Cancel", ButtonKind::Ghost, p).on_click(
                        cx.listener(|this, _, _, cx| this.state.read(cx).send(Command::CancelDownload)),
                    ),
                )
                .into_any_element()
        } else if !installed {
            button(SharedString::from(format!("dl-{id}")), format!("Download · {}", fmt_bytes(m.size_bytes())), ButtonKind::Primary, p)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.state.update(cx, |s, cx| {
                        s.send(Command::DownloadModel(id.to_string()));
                        if !s.installed.contains(s.config.model.id.as_str()) {
                            // First model: make it the active one.
                            s.edit_config(cx, |c| c.model.id = id.to_string());
                        }
                    });
                }))
                .into_any_element()
        } else {
            h_row()
                .gap(px(6.))
                .when(!active, |el| {
                    el.child(
                        button(SharedString::from(format!("use-{id}")), "Use", ButtonKind::Secondary, p).on_click(
                            cx.listener(move |this, _, _, cx| {
                                this.state.update(cx, |s, cx| s.edit_config(cx, |c| c.model.id = id.to_string()))
                            }),
                        ),
                    )
                    .child(
                        icon_button(SharedString::from(format!("rm-{id}")), I::Trash, p.text_muted, p).on_click(
                            cx.listener(move |this, _, _, cx| {
                                this.state.read(cx).send(Command::DeleteModel(id.to_string()))
                            }),
                        ),
                    )
                })
                .into_any_element()
        };

        v_col()
            .p(px(18.))
            .gap(px(12.))
            .rounded(px(12.))
            .bg(p.surface)
            .border_1()
            .border_color(if active { p.accent } else { p.border })
            .child(
                h_row()
                    .justify_between()
                    .gap(px(16.))
                    .child(
                        v_col()
                            .gap(px(4.))
                            .child(
                                h_row()
                                    .gap(px(10.))
                                    .child(
                                        div()
                                            .text_size(px(15.))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .child(m.name),
                                    )
                                    .when(m.id == models::DEFAULT_MODEL, |el| el.child(badge("Recommended", p.accent_text)))
                                    .child(status),
                            )
                            .child(div().text_size(px(13.)).text_color(p.text_muted).child(m.tagline)),
                    )
                    .child(actions),
            )
            .when_some(progress, |el, (done, total)| {
                el.child(progress_bar(if total > 0 { done as f32 / total as f32 } else { 0.0 }, p))
            })
            .child(
                h_row()
                    .gap(px(24.))
                    .child(spec("Accuracy", meter(m.accuracy, p).into_any_element(), p))
                    .child(spec("Speed", meter(m.speed, p).into_any_element(), p))
                    .child(spec(
                        "Languages",
                        div().text_size(px(12.5)).text_color(p.text).child(m.languages).into_any_element(),
                        p,
                    ))
                    .child(spec(
                        "Size",
                        div().text_size(px(12.5)).text_color(p.text).child(fmt_bytes(m.size_bytes())).into_any_element(),
                        p,
                    )),
            )
            .when_some(error, |el, e| {
                el.child(div().text_size(px(12.5)).text_color(p.danger).child(format!("Download failed: {e}")))
            })
            .when(active, |el| {
                el.when_some(
                    match engine {
                        EngineStatus::Failed { error, .. } => Some(error.clone()),
                        _ => None,
                    },
                    |el, e| el.child(div().text_size(px(12.5)).text_color(p.danger).child(e)),
                )
            })
    }
}

fn spec(label: &str, value: AnyElement, p: Palette) -> impl IntoElement {
    v_col()
        .gap(px(6.))
        .child(div().text_size(px(11.5)).text_color(p.text_faint).child(label.to_string()))
        .child(div().h(px(16.)).flex().items_center().child(value))
}

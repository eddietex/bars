use gpui_kit::assets::IconName as I;
use gpui_kit::component::input::Input;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use bars_core::controller::Command;
use bars_core::history::Entry;

use super::SettingsView;
use crate::theme::PaletteExt;
use crate::widgets::*;

impl SettingsView {
    pub fn render_history(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = cx.palette();
        let query = self.search.read(cx).value().to_lowercase();
        let st = self.state.read(cx);
        let save = st.config.ui.save_history;
        let entries: Vec<Entry> = st
            .history
            .iter()
            .rev()
            .filter(|e| query.is_empty() || e.text.to_lowercase().contains(&query))
            .take(500)
            .cloned()
            .collect();

        // Group by calendar day.
        let today = chrono::Local::now().date_naive();
        let mut groups: Vec<(String, Vec<Entry>)> = Vec::new();
        for e in entries {
            let day = e.at.date_naive();
            let label = if day == today {
                "Today".to_string()
            } else if Some(day) == today.pred_opt() {
                "Yesterday".to_string()
            } else {
                e.at.format("%A, %B %-d").to_string()
            };
            match groups.last_mut() {
                Some((l, v)) if *l == label => v.push(e),
                _ => groups.push((label, vec![e])),
            }
        }

        let state = self.state.clone();
        v_col()
            .child(page_header("History", Some("Stored only on this computer."), p))
            .child(
                h_row()
                    .gap(px(10.))
                    .mb(px(20.))
                    .child(div().flex_1().child(Input::new(&self.search).prefix(icon(I::Search, p.text_faint, 14.))))
                    .child(
                        h_row()
                            .gap(px(8.))
                            .child(div().text_size(px(12.5)).text_color(p.text_muted).child("Save history"))
                            .child(small_switch("save-history", save, {
                                let state = state.clone();
                                move |v, _, cx| {
                                    let v = *v;
                                    state.update(cx, |s, cx| s.edit_config(cx, |c| c.ui.save_history = v))
                                }
                            })),
                    )
                    .child(
                        button("clear-history", "Clear all", ButtonKind::Danger, p)
                            .on_click(cx.listener(|this, _, _, cx| this.state.read(cx).send(Command::ClearHistory))),
                    ),
            )
            .when(groups.is_empty(), |el| {
                el.child(
                    card(p)
                        .p(px(28.))
                        .items_center()
                        .child(div().text_size(px(13.5)).text_color(p.text_muted).child(if query.is_empty() {
                            "Nothing here yet."
                        } else {
                            "No matches."
                        })),
                )
            })
            .children(groups.into_iter().map(|(label, items)| {
                let rows_el = rows(p, items.iter().map(|e| self.entry_row(e, p, cx).into_any_element()).collect());
                section(&label, p).child(rows_el)
            }))
    }
}

//! Small, consistent building blocks used across the settings UI.

use gpui_kit::component::{Icon, IconNamed, Sizable};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::theme::Palette;

pub type ClickHandler = Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>;

/// Keyboard keycaps for a shortcut: outlined keys with a heavier bottom
/// edge (like a physical key) joined by "+", e.g. [Ctrl] + [Super].
pub fn keycaps(parts: Vec<String>, p: Palette, large: bool) -> impl IntoElement {
    let n = parts.len();
    let mut row = h_row().gap(px(if large { 8. } else { 5. }));
    for (i, part) in parts.into_iter().enumerate() {
        row = row.child(
            div()
                .px(px(if large { 13. } else { 7. }))
                .h(px(if large { 36. } else { 22. }))
                .min_w(px(if large { 36. } else { 22. }))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(if large { 8. } else { 5. }))
                .border_1()
                .border_b(px(if large { 3. } else { 2. }))
                .border_color(p.text_faint)
                .text_size(px(if large { 14. } else { 11.5 }))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(p.text)
                .child(part),
        );
        if i + 1 < n {
            row = row.child(
                div()
                    .text_size(px(if large { 15. } else { 11. }))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(p.text_muted)
                    .child("+"),
            );
        }
    }
    row
}

pub fn h_row() -> Div {
    div().flex().flex_row().items_center()
}

pub fn v_col() -> Div {
    div().flex().flex_col()
}

pub fn icon(name: impl IconNamed, color: Hsla, size: f32) -> Icon {
    Icon::new(Icon::default().path(name.path()))
        .text_color(color)
        .size(px(size))
}

/// Page heading with an optional subtitle.
pub fn page_header(title: &str, subtitle: Option<&str>, p: Palette) -> impl IntoElement {
    v_col()
        .gap(px(4.))
        .mb(px(24.))
        .child(
            div()
                .text_size(px(22.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(p.text)
                .child(title.to_string()),
        )
        .when_some(subtitle, |el, s| {
            el.child(div().text_size(px(13.5)).text_color(p.text_muted).child(s.to_string()))
        })
}

/// A titled group of setting rows on a card.
pub fn section(title: &str, p: Palette) -> Div {
    v_col().gap(px(8.)).mb(px(24.)).child(
        div()
            .px(px(4.))
            .text_size(px(12.5))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(p.text_muted)
            .child(title.to_string()),
    )
}

pub fn card(p: Palette) -> Div {
    v_col()
        .rounded(px(12.))
        .bg(p.surface)
        .border_1()
        .border_color(p.border)
        .overflow_hidden()
}

/// A setting row: label + description on the left, control on the right.
pub fn row(title: &str, description: Option<&str>, control: impl IntoElement, p: Palette) -> Div {
    h_row()
        .gap(px(24.))
        .px(px(18.))
        .py(px(14.))
        .justify_between()
        .child(
            v_col()
                .flex_1()
                .min_w_0()
                .gap(px(3.))
                .child(
                    div()
                        .text_size(px(14.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(p.text)
                        .child(title.to_string()),
                )
                .when_some(description, |el, d| {
                    el.child(
                        div()
                            .text_size(px(12.5))
                            .line_height(px(18.))
                            .text_color(p.text_muted)
                            .child(d.to_string()),
                    )
                }),
        )
        .child(div().flex_none().max_w(px(420.)).child(control))
}

pub fn divider(p: Palette) -> Div {
    div().h(px(1.)).w_full().bg(p.border)
}

/// Rows separated by hairlines inside a card.
pub fn rows(p: Palette, items: Vec<AnyElement>) -> Div {
    let n = items.len();
    let mut c = card(p);
    for (i, item) in items.into_iter().enumerate() {
        c = c.child(item);
        if i + 1 < n {
            c = c.child(div().px(px(18.)).child(divider(p)));
        }
    }
    c
}

/// Pill-shaped segmented control.
pub fn segmented<T: Copy + PartialEq + 'static>(
    id: impl Into<ElementId>,
    options: Vec<(T, &'static str)>,
    selected: T,
    p: Palette,
    on_select: impl Fn(T, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let on_select = std::rc::Rc::new(on_select);
    let id: ElementId = id.into();
    h_row()
        .p(px(3.))
        .gap(px(2.))
        .rounded(px(9.))
        .bg(p.surface_2)
        .border_1()
        .border_color(p.border)
        .children(options.into_iter().enumerate().map(move |(i, (value, label))| {
            let active = value == selected;
            let on_select = on_select.clone();
            div()
                .id(ElementId::NamedInteger(format!("{id}").into(), i as u64))
                .px(px(11.))
                .h(px(26.))
                .flex()
                .items_center()
                .rounded(px(6.))
                .text_size(px(12.5))
                .font_weight(FontWeight::MEDIUM)
                .cursor_pointer()
                .when(active, |el| {
                    el.bg(if p.dark { p.border_strong } else { p.surface })
                        .text_color(p.text)
                        .shadow_sm()
                })
                .when(!active, |el| {
                    el.text_color(p.text_muted).hover(|s| s.text_color(p.text))
                })
                .on_click(move |_, window, cx| on_select(value, window, cx))
                .child(label)
        }))
}

#[derive(Copy, Clone, PartialEq, Eq)]
pub enum ButtonKind {
    Primary,
    Secondary,
    Ghost,
    Danger,
    /// Soft accent tint: a clear action that isn't the page's main one.
    Accent,
}

pub fn button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    kind: ButtonKind,
    p: Palette,
) -> Stateful<Div> {
    let label: SharedString = label.into();
    let (bg, fg, hover) = match kind {
        ButtonKind::Primary => (p.accent, p.on_accent, p.accent_text),
        ButtonKind::Secondary => (p.surface_2, p.text, p.border_strong),
        ButtonKind::Ghost => (transparent_black(), p.text_muted, p.hover),
        ButtonKind::Danger => (transparent_black(), p.danger, p.hover),
        ButtonKind::Accent => (p.accent_soft, p.accent_text, {
            let mut h = p.accent_soft;
            h.a *= 1.7;
            h
        }),
    };
    div()
        .id(id.into())
        .h(px(30.))
        .px(px(12.))
        .flex()
        .items_center()
        .justify_center()
        .gap(px(6.))
        .rounded(px(8.))
        .bg(bg)
        .when(kind == ButtonKind::Secondary, |el| el.border_1().border_color(p.border))
        .text_color(fg)
        .text_size(px(13.))
        .font_weight(FontWeight::MEDIUM)
        .cursor_pointer()
        .hover(move |s| s.bg(hover))
        .when(!label.is_empty(), |el| el.child(label))
}

/// A button with a leading icon.
pub fn icon_label_button(
    id: impl Into<ElementId>,
    name: impl IconNamed,
    label: impl Into<SharedString>,
    kind: ButtonKind,
    p: Palette,
) -> Stateful<Div> {
    let color = match kind {
        ButtonKind::Primary => p.on_accent,
        ButtonKind::Accent => p.accent_text,
        ButtonKind::Danger => p.danger,
        _ => p.text_muted,
    };
    button(id, "", kind, p).child(icon(name, color, 14.)).child(label.into())
}

pub fn icon_button(id: impl Into<ElementId>, name: impl IconNamed, tooltip_color: Hsla, p: Palette) -> Stateful<Div> {
    div()
        .id(id.into())
        .size(px(28.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(7.))
        .cursor_pointer()
        .hover(move |s| s.bg(p.hover))
        .child(icon(name, tooltip_color, 15.))
}

pub fn badge(text: impl Into<SharedString>, color: Hsla) -> impl IntoElement {
    let mut bg = color;
    bg.a = 0.14;
    h_row()
        .h(px(20.))
        .px(px(8.))
        .rounded_full()
        .bg(bg)
        .text_color(color)
        .text_size(px(11.5))
        .font_weight(FontWeight::MEDIUM)
        .child(text.into())
}

/// Five small bars showing a 1-5 rating.
pub fn meter(value: u8, p: Palette) -> impl IntoElement {
    h_row().gap(px(3.)).children((1..=5u8).map(move |i| {
        div()
            .w(px(12.))
            .h(px(4.))
            .rounded_full()
            .bg(if i <= value { p.accent } else { p.border_strong })
    }))
}

pub fn stat(label: &str, value: String, hint: Option<String>, p: Palette) -> impl IntoElement {
    v_col()
        .flex_1()
        .min_w_0()
        .gap(px(6.))
        .p(px(16.))
        .rounded(px(12.))
        .bg(p.surface)
        .border_1()
        .border_color(p.border)
        .child(div().text_size(px(12.)).text_color(p.text_muted).child(label.to_string()))
        .child(
            div()
                .text_size(px(24.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(p.text)
                .child(value),
        )
        .when_some(hint, |el, h| el.child(div().text_size(px(11.5)).text_color(p.text_faint).child(h)))
}

pub fn progress_bar(fraction: f32, p: Palette) -> impl IntoElement {
    div()
        .h(px(4.))
        .w_full()
        .rounded_full()
        .bg(p.border)
        .child(
            div()
                .h_full()
                .rounded_full()
                .bg(p.accent)
                .w(relative(fraction.clamp(0.0, 1.0))),
        )
}

pub fn fmt_bytes(b: u64) -> String {
    let mb = b as f64 / 1_000_000.0;
    if mb >= 1000.0 {
        format!("{:.1} GB", mb / 1000.0)
    } else {
        format!("{mb:.0} MB")
    }
}

pub fn fmt_number(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, ch) in s.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    out.chars().rev().collect()
}

pub fn small_switch(
    id: impl Into<ElementId>,
    checked: bool,
    on_change: impl Fn(&bool, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    gpui_kit::component::switch::Switch::new(id)
        .checked(checked)
        .small()
        .on_click(on_change)
}

#[allow(unused)]
pub fn handler(f: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> ClickHandler {
    Box::new(f)
}

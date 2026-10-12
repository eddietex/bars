//! Bars's visual language: a neutral, quiet base with one accent colour.
//! Tokens live in a GPUI global so every view reads the same palette, and are
//! mirrored onto gpui-component's theme so its widgets match.

use gpui_kit::component::{Theme, ThemeMode};
use gpui_kit::*;
use bars_core::config::ThemeMode as Pref;

pub const FONT: &str = "Inter";

#[derive(Clone, Copy, Debug)]
pub struct Palette {
    pub dark: bool,
    pub bg: Hsla,
    pub sidebar: Hsla,
    pub surface: Hsla,
    pub surface_2: Hsla,
    pub hover: Hsla,
    pub border: Hsla,
    pub border_strong: Hsla,
    pub text: Hsla,
    pub text_muted: Hsla,
    pub text_faint: Hsla,
    pub accent: Hsla,
    pub accent_soft: Hsla,
    pub accent_text: Hsla,
    pub on_accent: Hsla,
    pub recording: Hsla,
    pub success: Hsla,
    pub warning: Hsla,
    pub danger: Hsla,
}

impl Global for Palette {}

fn c(hex: u32) -> Hsla {
    rgb(hex).into()
}

fn ca(hex: u32, alpha: f32) -> Hsla {
    let mut h: Hsla = rgb(hex).into();
    h.a = alpha;
    h
}

impl Palette {
    pub fn dark() -> Self {
        Palette {
            dark: true,
            bg: c(0x0f0f12),
            sidebar: c(0x131317),
            surface: c(0x17171c),
            surface_2: c(0x1e1e24),
            hover: ca(0xffffff, 0.045),
            border: ca(0xffffff, 0.075),
            border_strong: ca(0xffffff, 0.13),
            text: c(0xededf0),
            text_muted: c(0x9a9aa6),
            text_faint: c(0x62626d),
            accent: c(0x8a5cff),
            accent_soft: ca(0x8a5cff, 0.15),
            accent_text: c(0xb79cff),
            on_accent: c(0xffffff),
            recording: c(0xff5a4e),
            success: c(0x3dd68c),
            warning: c(0xf5b84a),
            danger: c(0xff6369),
        }
    }

    pub fn light() -> Self {
        Palette {
            dark: false,
            bg: c(0xfbfbfc),
            sidebar: c(0xf4f4f6),
            surface: c(0xffffff),
            surface_2: c(0xf1f1f4),
            hover: ca(0x000000, 0.04),
            border: ca(0x000000, 0.08),
            border_strong: ca(0x000000, 0.14),
            text: c(0x131316),
            text_muted: c(0x63636e),
            text_faint: c(0x9a9aa5),
            accent: c(0x7040f0),
            accent_soft: ca(0x7040f0, 0.10),
            accent_text: c(0x5f30dc),
            on_accent: c(0xffffff),
            recording: c(0xe5484d),
            success: c(0x1f9d5f),
            warning: c(0xc77f0a),
            danger: c(0xdc3e42),
        }
    }
}

pub trait PaletteExt {
    fn palette(&self) -> Palette;
}

impl PaletteExt for App {
    fn palette(&self) -> Palette {
        *self.global::<Palette>()
    }
}

/// The user's light/dark preference, kept global so any window can follow
/// the system appearance.
#[derive(Copy, Clone)]
pub struct ThemePref(pub Pref);

impl Global for ThemePref {}

pub fn set_pref(pref: Pref, cx: &mut App) {
    cx.set_global(ThemePref(pref));
}

/// Re-applies the palette if the window's appearance and the preference
/// disagree with what's installed.
pub fn follow_system(window: &Window, cx: &mut App) {
    let pref = cx.try_global::<ThemePref>().map_or(Pref::System, |p| p.0);
    let dark = match pref {
        Pref::Dark => true,
        Pref::Light => false,
        Pref::System => matches!(window.appearance(), WindowAppearance::Dark | WindowAppearance::VibrantDark),
    };
    if cx.try_global::<Palette>().is_none_or(|p| p.dark != dark) {
        apply(dark, cx);
        cx.refresh_windows();
    }
}

/// Resolve the user's preference against the system appearance.
pub fn wants_dark(pref: Pref, cx: &App) -> bool {
    match pref {
        Pref::Dark => true,
        Pref::Light => false,
        Pref::System => matches!(
            cx.window_appearance(),
            WindowAppearance::Dark | WindowAppearance::VibrantDark
        ),
    }
}

/// Install the palette and restyle gpui-component widgets to match.
pub fn apply(dark: bool, cx: &mut App) {
    let p = if dark { Palette::dark() } else { Palette::light() };
    cx.set_global(p);
    Theme::change(if dark { ThemeMode::Dark } else { ThemeMode::Light }, None, cx);
    Theme::update(cx, |t| {
        t.font_family = FONT.into();
        t.font_size = px(14.);
        t.radius = px(8.);
        t.radius_lg = px(12.);
        t.shadow = false;
        let col = &mut t.colors;
        col.background = p.bg;
        col.foreground = p.text;
        col.border = p.border_strong;
        col.input = p.border_strong;
        col.ring = p.accent;
        col.primary = p.accent;
        col.primary_hover = p.accent_text;
        col.primary_active = p.accent;
        col.primary_foreground = p.on_accent;
        col.muted = p.surface_2;
        col.muted_foreground = p.text_muted;
        col.accent = p.hover;
        col.accent_foreground = p.text;
        col.popover = p.surface;
        col.popover_foreground = p.text;
        col.list = p.surface;
        col.list_hover = p.hover;
        col.list_active = p.accent_soft;
        col.list_active_border = p.accent;
        col.switch = p.border_strong;
        col.switch_thumb = c(0xffffff);
        col.slider_bar = p.accent;
        col.slider_thumb = c(0xffffff);
        col.progress_bar = p.accent;
        col.selection = p.accent_soft;
        col.caret = p.accent;
        col.scrollbar = ca(0x000000, 0.0);
        col.scrollbar_thumb = p.border_strong;
        col.scrollbar_thumb_hover = p.text_faint;
        col.title_bar = p.sidebar;
        col.title_bar_border = p.border;
        col.sidebar = p.sidebar;
        col.sidebar_border = p.border;
        col.overlay = ca(0x000000, if dark { 0.5 } else { 0.25 });
    });
}

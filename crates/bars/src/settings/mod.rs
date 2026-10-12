//! The settings window: a quiet sidebar and one page at a time.

mod dictionary;
mod general;
mod history;
mod home;
mod model;
mod setup;

use gpui_kit::assets::IconName as I;
use gpui_kit::component::input::{InputState, TextareaState};
use gpui_kit::component::TitleBar;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use bars_core::controller::Phase;
use bars_core::engine::EngineStatus;

use crate::state::{AppState, UpdateStatus};
use crate::theme::{FONT, PaletteExt};
use crate::widgets::{ButtonKind, button, h_row, icon, v_col};

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Page {
    Home,
    General,
    Model,
    Dictionary,
    History,
    Setup,
}

impl Page {
    /// Dev helper: `BARS_PAGE=model bars` opens straight to a page.
    pub fn from_env() -> Option<Page> {
        let v = std::env::var("BARS_PAGE").ok()?;
        Some(match v.as_str() {
            "home" => Page::Home,
            "general" => Page::General,
            "model" => Page::Model,
            "dictionary" => Page::Dictionary,
            "history" => Page::History,
            "setup" => Page::Setup,
            _ => return None,
        })
    }

    fn label(self) -> &'static str {
        match self {
            Page::Home => "Home",
            Page::General => "Shortcut & input",
            Page::Model => "Speech model",
            Page::Dictionary => "Dictionary",
            Page::History => "History",
            Page::Setup => "Setup",
        }
    }

    fn icon(self) -> I {
        match self {
            Page::Home => I::House,
            Page::General => I::Keyboard,
            Page::Model => I::AudioLines,
            Page::Dictionary => I::BookA,
            Page::History => I::ListClock,
            Page::Setup => I::ShieldCheck,
        }
    }
}

pub struct SettingsView {
    pub state: Entity<AppState>,
    pub page: Page,
    // Dictionary page inputs.
    pub dict_from: Entity<InputState>,
    pub dict_to: Entity<InputState>,
    pub snip_trigger: Entity<InputState>,
    pub snip_text: Entity<InputState>,
    // History page search.
    pub search: Entity<InputState>,
    // Setup page practice field.
    pub practice: Entity<InputState>,
    pub copied: Option<u64>,
    /// A History entry being corrected, and its editor.
    pub editing: Option<(u64, Entity<TextareaState>)>,
    _subs: Vec<Subscription>,
}

impl SettingsView {
    pub fn new(state: Entity<AppState>, page: Page, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mk = |placeholder: &'static str, window: &mut Window, cx: &mut Context<Self>| {
            cx.new(|cx| InputState::new(window, cx).placeholder(placeholder))
        };
        let search = mk("Search history", window, cx);
        window.set_window_title("Bars");
        let subs = vec![
            cx.observe(&state, |_, _, cx| cx.notify()),
            cx.observe(&search, |_, _, cx| cx.notify()),
            // Follow the desktop's light/dark preference live.
            cx.observe_window_appearance(window, |this, window, cx| {
                this.sync_theme(window, cx);
            }),
        ];
        SettingsView {
            dict_from: mk("What you say, e.g. “example dot x y z”", window, cx),
            dict_to: mk("What to write, e.g. example.xyz", window, cx),
            snip_trigger: mk("Spoken cue, e.g. “my address”", window, cx),
            snip_text: mk("Text to insert", window, cx),
            practice: mk("Click here, then hold your shortcut and speak…", window, cx),
            search,
            state,
            page,
            copied: None,
            editing: None,
            _subs: subs,
        }
    }

    pub fn sync_theme(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        crate::theme::follow_system(window, cx);
    }

    pub fn go(&mut self, page: Page, cx: &mut Context<Self>) {
        self.page = page;
        if page == Page::General {
            self.state.update(cx, |s, cx| s.refresh_devices(cx));
        }
        cx.notify();
    }

    fn render_sidebar(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let p = cx.palette();
        let st = self.state.read(cx);
        let needs_setup = !st.setup_complete();
        let status = status_line(st);
        let update = st.update.offered().map(|r| (r.version.clone(), st.update.clone()));
        let pages = [Page::Home, Page::General, Page::Model, Page::Dictionary, Page::History];
        let item = |page: Page, attention: bool, cx: &mut Context<Self>| {
            let active = self.page == page;
            div()
                .id(SharedString::from(page.label()))
                .h(px(34.))
                .px(px(10.))
                .flex()
                .items_center()
                .gap(px(10.))
                .rounded(px(8.))
                .cursor_pointer()
                .text_size(px(13.5))
                .font_weight(FontWeight::MEDIUM)
                .when(active, |el| el.bg(p.hover).text_color(p.text))
                .when(!active, |el| el.text_color(p.text_muted).hover(|s| s.bg(p.hover).text_color(p.text)))
                .on_click(cx.listener(move |this, _, _, cx| this.go(page, cx)))
                .child(icon(page.icon(), if active { p.accent_text } else { p.text_faint }, 16.))
                .child(div().flex_1().child(page.label()))
                .when(attention, |el| el.child(div().size(px(7.)).rounded_full().bg(p.warning)))
        };

        v_col()
            .w(px(232.))
            .h_full()
            .flex_none()
            .bg(p.sidebar)
            .border_r_1()
            .border_color(p.border)
            .px(px(12.))
            .pb(px(14.))
            .child(
                h_row()
                    .px(px(2.))
                    .pt(px(14.))
                    .pb(px(24.))
                    .child(crate::brand::wordmark(204., p.dark)),
            )
            .child(
                v_col()
                    .gap(px(2.))
                    .children(pages.into_iter().map(|page| item(page, false, cx).into_any_element())),
            )
            .child(div().flex_1())
            .when_some(update, |el, (version, status)| el.child(self.update_banner(version, status, cx)))
            .child(item(Page::Setup, needs_setup, cx))
            .child(
                h_row()
                    .mt(px(12.))
                    .px(px(10.))
                    .gap(px(8.))
                    .child(div().size(px(7.)).rounded_full().bg(match status.1 {
                        Status::Good => p.success,
                        Status::Busy => p.accent,
                        Status::Warn => p.warning,
                        Status::Live => p.recording,
                    }))
                    .child(div().flex_1().text_size(px(12.)).text_color(p.text_muted).child(status.0))
                    .child(
                        div()
                            .text_size(px(11.5))
                            .text_color(p.text_faint)
                            .child(concat!("v", env!("CARGO_PKG_VERSION"))),
                    ),
            )
    }
}

impl SettingsView {
    /// Above Setup while a newer Bars is on offer: what it is, and a button
    /// to update (or download, where Bars can't install it itself).
    fn update_banner(&self, version: String, status: UpdateStatus, cx: &mut Context<Self>) -> impl IntoElement {
        let p = cx.palette();
        let (detail, action) = match status {
            UpdateStatus::Installing(_) => (format!("Installing ver. {version}…"), None),
            UpdateStatus::InstallFailed(..) => ("Couldn't install".to_string(), Some("Try again")),
            _ => (format!("ver. {version}"), Some(crate::shell::update_action())),
        };
        let state = self.state.clone();
        v_col()
            .mb(px(8.))
            .px(px(12.))
            .py(px(10.))
            .gap(px(8.))
            .rounded(px(10.))
            .bg(p.accent_soft)
            .child(
                h_row()
                    .gap(px(8.))
                    .text_size(px(13.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(p.text)
                    .child(icon(I::Sparkles, p.accent_text, 15.))
                    .child("New version available"),
            )
            .child(
                h_row()
                    .gap(px(8.))
                    .child(div().flex_1().text_size(px(12.)).text_color(p.text_muted).child(detail))
                    .when_some(action, |el, label| {
                        el.child(button("sidebar-update", label, ButtonKind::Primary, p).h(px(26.)).on_click(
                            move |_, _, cx| state.update(cx, |s, cx| s.install_update(cx)),
                        ))
                    }),
            )
    }
}

#[derive(Clone, Copy)]
enum Status {
    Good,
    Busy,
    Warn,
    Live,
}

fn status_line(st: &AppState) -> (String, Status) {
    match st.phase {
        Phase::Recording { .. } => return ("Listening…".into(), Status::Live),
        Phase::Transcribing => return ("Transcribing…".into(), Status::Busy),
        Phase::Idle => {}
    }
    if let Some((id, (done, total))) = st.downloads.iter().next() {
        let name = bars_core::models::find(id).map_or("model", |m| m.name);
        let pct = if *total > 0 { done * 100 / total } else { 0 };
        return (format!("Downloading {name} · {pct}%"), Status::Busy);
    }
    match &st.engine {
        EngineStatus::Loading(_) => ("Loading model…".into(), Status::Busy),
        EngineStatus::Ready { .. } | EngineStatus::Unloaded(_) if st.setup_complete() => {
            ("Ready".into(), Status::Good)
        }
        EngineStatus::Ready { .. } | EngineStatus::Unloaded(_) => ("Finish setup".into(), Status::Warn),
        EngineStatus::Missing(_) => ("Model not downloaded".into(), Status::Warn),
        EngineStatus::Failed { .. } => ("Model failed to load".into(), Status::Warn),
    }
}

impl Render for SettingsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = cx.palette();
        let page: AnyElement = match self.page {
            Page::Home => self.render_home(window, cx).into_any_element(),
            Page::General => self.render_general(window, cx).into_any_element(),
            Page::Model => self.render_model(window, cx).into_any_element(),
            Page::Dictionary => self.render_dictionary(window, cx).into_any_element(),
            Page::History => self.render_history(window, cx).into_any_element(),
            Page::Setup => self.render_setup(window, cx).into_any_element(),
        };
        v_col()
            .size_full()
            .bg(p.bg)
            .font_family(FONT)
            .text_color(p.text)
            .child(TitleBar::new().bg(p.sidebar).border_color(p.border))
            .child(
                h_row()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .items_start()
                    .child(self.render_sidebar(cx))
                    .child(
                        div()
                            .id("page-scroll")
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            .overflow_y_scroll()
                            .child(
                                div()
                                    .w_full()
                                    .max_w(px(780.))
                                    .mx_auto()
                                    .px(px(40.))
                                    .pt(px(28.))
                                    .pb(px(48.))
                                    .child(page),
                            ),
                    ),
            )
    }
}

pub fn open(state: Entity<AppState>, page: Page, cx: &mut App) -> anyhow::Result<WindowHandle<gpui_kit::base::Root>> {
    let bounds = Bounds::centered(None, size(px(980.), px(680.)), cx);
    let opts = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        window_min_size: Some(size(px(760.), px(520.))),
        window_decorations: Some(WindowDecorations::Client),
        app_id: Some(bars_core::paths::APP_ID.into()),
        icon: crate::brand::window_icon(),
        focus: true,
        show: true,
        ..TitleBar::window_options()
    };
    let (handle, _) = gpui_kit::open_window(opts, cx, |window, cx| {
        cx.new(|cx| {
            let mut view = SettingsView::new(state, page, window, cx);
            view.sync_theme(window, cx);
            view
        })
    })?;
    handle
        .downcast::<gpui_kit::base::Root>()
        .ok_or_else(|| anyhow::anyhow!("unexpected root type"))
}

//! The Bars brand assets: the wordmark (sidebar, setup assistant), the app
//! icon (window icon) and the tray glyph. Sources live in packaging/artwork.

use std::borrow::Cow;
use std::sync::Arc;

use gpui_kit::*;

const ICON_PNG: &[u8] = include_bytes!("../assets/brand/icon-256.png");
const WORDMARK_DARK_PATH: &str = "brand/wordmark-dark.png";
const WORDMARK_DARK_PNG: &[u8] = include_bytes!("../assets/brand/wordmark-dark.png");
const WORDMARK_LIGHT_PATH: &str = "brand/wordmark-light.png";
const WORDMARK_LIGHT_PNG: &[u8] = include_bytes!("../assets/brand/wordmark-light.png");
/// Width / height of the wordmark artwork.
const WORDMARK_ASPECT: f32 = 3.266;

/// Serves the brand images to GPUI's asset loader.
pub fn load_asset(path: &str) -> Option<Cow<'static, [u8]>> {
    match path {
        WORDMARK_DARK_PATH => Some(Cow::Borrowed(WORDMARK_DARK_PNG)),
        WORDMARK_LIGHT_PATH => Some(Cow::Borrowed(WORDMARK_LIGHT_PNG)),
        _ => None,
    }
}

/// The "Bars" wordmark (mic, bars and lettering), in the variant that
/// suits the current theme.
pub fn wordmark(width: f32, dark: bool) -> impl IntoElement {
    img(if dark { WORDMARK_DARK_PATH } else { WORDMARK_LIGHT_PATH })
        .w(px(width))
        .h(px(width / WORDMARK_ASPECT))
        .flex_none()
}

/// The window icon (shown by compositors that support per-window icons).
pub fn window_icon() -> Option<Arc<image::RgbaImage>> {
    image::load_from_memory_with_format(ICON_PNG, image::ImageFormat::Png)
        .map(|i| Arc::new(i.to_rgba8()))
        .map_err(|e| log::warn!("window icon: {e}"))
        .ok()
}

/// Brand gradient endpoints, sampled from the icon's bars.
pub const GRADIENT_TOP: u32 = 0xc42dfe;
pub const GRADIENT_BOTTOM: u32 = 0x2a6cfc;

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum TrayState {
    Idle,
    Recording,
    Busy,
}

macro_rules! tray_set {
    ($state:literal) => {
        [
            (22, include_bytes!(concat!("../assets/tray/", $state, "-22.argb")).as_slice()),
            (32, include_bytes!(concat!("../assets/tray/", $state, "-32.argb")).as_slice()),
            (48, include_bytes!(concat!("../assets/tray/", $state, "-48.argb")).as_slice()),
            (64, include_bytes!(concat!("../assets/tray/", $state, "-64.argb")).as_slice()),
        ]
    };
}

const TRAY_IDLE: [(i32, &[u8]); 4] = tray_set!("idle");
const TRAY_RECORDING: [(i32, &[u8]); 4] = tray_set!("recording");
const TRAY_BUSY: [(i32, &[u8]); 4] = tray_set!("busy");

/// Pre-rendered tray icons as (size, ARGB32 network-order pixels), the
/// format the StatusNotifierItem spec expects. Recording adds a red dot,
/// transcribing an amber one.
pub fn tray_pixmaps(state: TrayState) -> &'static [(i32, &'static [u8])] {
    match state {
        TrayState::Idle => &TRAY_IDLE,
        TrayState::Recording => &TRAY_RECORDING,
        TrayState::Busy => &TRAY_BUSY,
    }
}

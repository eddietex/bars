//! Windows/macOS overlay behaviour GPUI doesn't expose: clicks pass through
//! everything but the pill, and the pill shows up on the screen you're
//! working on.
//!
//! Calls that make the OS notify the window (moves, resizes) run on a
//! foreground task, outside GPUI's render pass, so its callbacks can't
//! re-enter the app while it is borrowed.

use gpui_kit::*;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

#[derive(Default)]
pub struct NativeOverlay {
    /// The clickable area last applied (Windows window region).
    #[cfg(target_os = "windows")]
    region: Option<Option<Bounds<Pixels>>>,
    /// The window whose activation behaviour was adjusted.
    #[cfg(target_os = "windows")]
    hooked: Option<isize>,
    /// Whether the window currently lets clicks through (macOS).
    #[cfg(target_os = "macos")]
    ignoring: Option<bool>,
}

fn raw(window: &Window) -> Option<RawWindowHandle> {
    // GPUI's inherent `window_handle` returns its own handle type.
    HasWindowHandle::window_handle(window).ok().map(|h| h.as_raw())
}

impl NativeOverlay {
    /// Keeps the window clickable only over `pill` (nothing when `None`).
    pub fn set_clickable(&mut self, window: &Window, pill: Option<Bounds<Pixels>>, cx: &App) {
        #[cfg(target_os = "windows")]
        {
            let Some(hwnd) = win::hwnd(window) else { return };
            if self.hooked != Some(hwnd) {
                self.hooked = Some(hwnd);
                self.region = None;
                win::never_activate(hwnd);
            }
            if self.region == Some(pill) {
                return;
            }
            self.region = Some(pill);
            let scale = window.scale_factor();
            cx.foreground_executor()
                .spawn(async move { win::set_region(hwnd, pill, scale) })
                .detach();
        }
        #[cfg(target_os = "macos")]
        {
            let _ = cx;
            let Some(ns_window) = mac::ns_window(window) else { return };
            // The pointer has to be checked every frame: while the window
            // ignores the mouse it gets no events to tell it otherwise.
            let ignore = !pill.is_some_and(|b| mac::pointer_in(&ns_window, b));
            if self.ignoring != Some(ignore) {
                self.ignoring = Some(ignore);
                ns_window.setIgnoresMouseEvents(ignore);
            }
        }
    }

    /// Moves the window to the bottom centre of the work area (above the
    /// taskbar or Dock) of the screen with the pointer.
    pub fn place_on_active_screen(&mut self, window: &Window, size: Size<Pixels>, margin: f32, cx: &App) {
        #[cfg(target_os = "windows")]
        {
            let Some(hwnd) = win::hwnd(window) else { return };
            cx.foreground_executor()
                .spawn(async move { win::place(hwnd, size, margin) })
                .detach();
        }
        #[cfg(target_os = "macos")]
        {
            let Some(ns_window) = mac::ns_window(window) else { return };
            cx.foreground_executor()
                .spawn(async move { mac::place(&ns_window, size, margin) })
                .detach();
        }
    }
}

#[cfg(target_os = "windows")]
mod win {
    use std::sync::atomic::{AtomicIsize, Ordering};

    use gpui_kit::*;
    use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
    use windows::Win32::Graphics::Gdi::{
        CreateRectRgn, CreateRoundRectRgn, GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint,
        SetWindowRgn,
    };
    use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
    use windows::Win32::UI::WindowsAndMessaging::{
        CallWindowProcW, GWLP_WNDPROC, GetCursorPos, GetWindowLongPtrW, HWND_TOPMOST, MA_NOACTIVATE, SWP_NOACTIVATE, SetWindowLongPtrW,
        SetWindowPos, WM_MOUSEACTIVATE, WNDPROC,
    };

    use super::raw;

    pub fn hwnd(window: &Window) -> Option<isize> {
        match raw(window)? {
            raw_window_handle::RawWindowHandle::Win32(h) => Some(h.hwnd.get()),
            _ => None,
        }
    }

    fn to_hwnd(h: isize) -> HWND {
        HWND(h as *mut core::ffi::c_void)
    }

    /// GPUI's window procedure, which the overlay forwards everything else to.
    static GPUI_PROC: AtomicIsize = AtomicIsize::new(0);

    unsafe extern "system" fn overlay_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        // Clicking the pill must not take focus from the app being typed into.
        if msg == WM_MOUSEACTIVATE {
            return LRESULT(MA_NOACTIVATE as isize);
        }
        let original: WNDPROC = unsafe { std::mem::transmute(GPUI_PROC.load(Ordering::Relaxed)) };
        unsafe { CallWindowProcW(original, hwnd, msg, wparam, lparam) }
    }

    /// GPUI activates windows on click; the overlay never should.
    pub fn never_activate(hwnd: isize) {
        let ours = overlay_proc as unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> LRESULT as isize;
        unsafe {
            let current = GetWindowLongPtrW(to_hwnd(hwnd), GWLP_WNDPROC);
            if current == 0 || current == ours {
                return;
            }
            GPUI_PROC.store(current, Ordering::Relaxed);
            SetWindowLongPtrW(to_hwnd(hwnd), GWLP_WNDPROC, ours);
        }
    }

    /// Window region: the pill (with room for its edge and shadow), or
    /// nothing. Outside it, clicks go to the windows below.
    pub fn set_region(hwnd: isize, pill: Option<Bounds<Pixels>>, scale: f32) {
        let px = |v: f32| (v * scale).round() as i32;
        unsafe {
            let region = match pill {
                Some(b) => {
                    let (x, y) = (f32::from(b.origin.x), f32::from(b.origin.y));
                    let (w, h) = (f32::from(b.size.width), f32::from(b.size.height));
                    CreateRoundRectRgn(px(x - 6.), px(y - 4.), px(x + w + 6.), px(y + h + 10.), px(h + 10.), px(h + 10.))
                }
                None => CreateRectRgn(0, 0, 0, 0),
            };
            // The system owns the region from here on.
            SetWindowRgn(to_hwnd(hwnd), Some(region), true);
        }
    }

    pub fn place(hwnd: isize, size: Size<Pixels>, margin: f32) {
        unsafe {
            let mut pt = POINT::default();
            if GetCursorPos(&mut pt).is_err() {
                return;
            }
            let monitor = MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST);
            let mut info = MONITORINFO {
                cbSize: std::mem::size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };
            if !GetMonitorInfoW(monitor, &mut info).as_bool() {
                return;
            }
            let (mut dpi, mut dpi_y) = (96u32, 96u32);
            let _ = GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi, &mut dpi_y);
            let scale = dpi as f32 / 96.0;
            let w = (f32::from(size.width) * scale).round() as i32;
            let h = (f32::from(size.height) * scale).round() as i32;
            let work = info.rcWork;
            let x = work.left + (work.right - work.left - w) / 2;
            let y = work.bottom - h - (margin * scale).round() as i32;
            // Twice: moving onto a monitor with another scale makes GPUI
            // resize the window mid-move; the second call settles it.
            for _ in 0..2 {
                let _ = SetWindowPos(to_hwnd(hwnd), Some(HWND_TOPMOST), x, y, w, h, SWP_NOACTIVATE);
            }
        }
    }
}

#[cfg(target_os = "macos")]
mod mac {
    use gpui_kit::*;
    use objc2::rc::Retained;
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSEvent, NSScreen, NSView, NSWindow};
    use objc2_foundation::{NSPoint, NSRect};

    use super::raw;

    pub fn ns_window(window: &Window) -> Option<Retained<NSWindow>> {
        match raw(window)? {
            raw_window_handle::RawWindowHandle::AppKit(h) => {
                let view: &NSView = unsafe { h.ns_view.cast().as_ref() };
                view.window()
            }
            _ => None,
        }
    }

    fn contains(r: NSRect, p: NSPoint) -> bool {
        p.x >= r.origin.x && p.x < r.origin.x + r.size.width && p.y >= r.origin.y && p.y < r.origin.y + r.size.height
    }

    /// Whether the pointer is over `pill` (window coordinates, top-left
    /// origin), with a little slack around its edge.
    pub fn pointer_in(window: &NSWindow, pill: Bounds<Pixels>) -> bool {
        let mouse = NSEvent::mouseLocation();
        let frame = window.frame();
        // Screen coordinates have a bottom-left origin.
        let x = (mouse.x - frame.origin.x) as f32;
        let y = (frame.origin.y + frame.size.height - mouse.y) as f32;
        let slack = 3.0;
        x >= f32::from(pill.origin.x) - slack
            && x <= f32::from(pill.origin.x + pill.size.width) + slack
            && y >= f32::from(pill.origin.y) - slack
            && y <= f32::from(pill.origin.y + pill.size.height) + slack
    }

    pub fn place(window: &NSWindow, size: Size<Pixels>, margin: f32) {
        let Some(mtm) = MainThreadMarker::new() else { return };
        let mouse = NSEvent::mouseLocation();
        let screens = NSScreen::screens(mtm);
        let Some(screen) = screens.iter().find(|s| contains(s.frame(), mouse)).or_else(|| screens.firstObject())
        else {
            return;
        };
        let area = screen.visibleFrame();
        let w = f32::from(size.width) as f64;
        window.setFrameOrigin(NSPoint::new(
            area.origin.x + (area.size.width - w) / 2.0,
            area.origin.y + margin as f64,
        ));
    }
}

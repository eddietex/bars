//! The Windows clipboard as a [`LazyClipboard`]. Dictated text is placed
//! with delayed rendering: Windows asks for it (WM_RENDERFORMAT) only when
//! an app pastes, which proves the paste landed. It's marked to stay out
//! of clipboard history and cloud sync. Everything on the clipboard before
//! is saved and put back afterwards.
//!
//! Delayed rendering needs a window whose thread answers messages, so the
//! clipboard is driven from a thread of its own.

use std::cell::RefCell;
use std::sync::OnceLock;
use std::thread;
use std::time::Duration;

use anyhow::anyhow;
use crossbeam_channel::{Receiver, Sender, bounded, unbounded};
use windows::Win32::Foundation::{GlobalFree, HANDLE, HGLOBAL, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, EnumClipboardFormats, GetClipboardData, GetClipboardOwner,
    IsClipboardFormatAvailable, OpenClipboard, RegisterClipboardFormatW, SetClipboardData,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock};
use windows::Win32::System::Ole::CF_UNICODETEXT;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, HWND_MESSAGE, MSG, PostMessageW, RegisterClassW,
    WINDOW_EX_STYLE, WINDOW_STYLE, WM_APP, WM_DESTROYCLIPBOARD, WM_RENDERALLFORMATS, WM_RENDERFORMAT, WNDCLASSW,
};
use windows::core::{PCWSTR, w};

use crate::platform::paste::LazyClipboard;

/// Clipboard formats and their bytes.
pub type Saved = Vec<(u32, Vec<u8>)>;

/// Formats larger than this aren't kept (nor restored).
const MAX_FORMAT_BYTES: usize = 32 << 20;
/// Asks the clipboard thread to run queued jobs.
const WM_JOBS: u32 = WM_APP + 1;

enum Job {
    Save(Sender<Option<Saved>>),
    Offer(Vec<u16>, Sender<anyhow::Result<Receiver<()>>>),
    Restore(Saved, Sender<()>),
    StillOurs(Sender<bool>),
}

struct Thread {
    jobs: Sender<Job>,
    hwnd: isize,
}

fn clipboard_thread() -> anyhow::Result<&'static Thread> {
    static THREAD: OnceLock<Option<Thread>> = OnceLock::new();
    THREAD
        .get_or_init(|| {
            let (jobs, rx) = unbounded();
            let (ready_tx, ready_rx) = bounded(1);
            thread::Builder::new()
                .name("bars-clipboard".into())
                .spawn(move || run(rx, ready_tx))
                .ok()?;
            let hwnd = ready_rx.recv().ok()??;
            Some(Thread { jobs, hwnd })
        })
        .as_ref()
        .ok_or_else(|| anyhow!("the clipboard window couldn't be created"))
}

fn submit<T>(make: impl FnOnce(Sender<T>) -> Job) -> anyhow::Result<T> {
    let t = clipboard_thread()?;
    let (tx, rx) = bounded(1);
    t.jobs.send(make(tx)).map_err(|_| anyhow!("the clipboard thread stopped"))?;
    unsafe { PostMessageW(Some(HWND(t.hwnd as *mut _)), WM_JOBS, WPARAM(0), LPARAM(0))? };
    rx.recv_timeout(Duration::from_secs(3)).map_err(|_| anyhow!("the clipboard didn't answer"))
}

#[derive(Default)]
pub struct WinClipboard;

impl LazyClipboard for WinClipboard {
    type Saved = Saved;
    const ASKS_EVERY_PASTE: bool = false;

    fn save(&mut self) -> Option<Saved> {
        submit(Job::Save).ok().flatten()
    }

    fn offer(&mut self, text: &str) -> anyhow::Result<Receiver<()>> {
        let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
        submit(|tx| Job::Offer(wide, tx))?
    }

    fn still_ours(&mut self) -> bool {
        submit(Job::StillOurs).unwrap_or(false)
    }

    fn restore(&mut self, saved: Saved) {
        let _ = submit(|tx| Job::Restore(saved, tx));
    }
}

struct State {
    hwnd: HWND,
}

/// The offered text (UTF-16, NUL-terminated) and who hears of reads.
type Pending = Option<(Vec<u16>, Sender<()>)>;

thread_local! {
    static JOBS: RefCell<Option<Receiver<Job>>> = const { RefCell::new(None) };
    // Borrowed only briefly: clipboard calls send messages straight back
    // to this window (EmptyClipboard: WM_DESTROYCLIPBOARD).
    static PENDING: RefCell<Pending> = const { RefCell::new(None) };
}

fn set_pending(p: Pending) {
    PENDING.with(|cell| {
        if let Ok(mut cur) = cell.try_borrow_mut() {
            *cur = p;
        }
    });
}

fn pending() -> Pending {
    PENDING.with(|cell| cell.try_borrow().ok().and_then(|p| p.clone()))
}

fn run(jobs: Receiver<Job>, ready: Sender<Option<isize>>) {
    let hwnd = unsafe {
        let instance = GetModuleHandleW(None).ok();
        let class = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: instance.map(Into::into).unwrap_or_default(),
            lpszClassName: w!("BarsClipboard"),
            ..Default::default()
        };
        RegisterClassW(&class);
        CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("BarsClipboard"),
            PCWSTR::null(),
            WINDOW_STYLE(0),
            0,
            0,
            0,
            0,
            Some(HWND_MESSAGE),
            None,
            instance.map(Into::into),
            None,
        )
    };
    let Ok(hwnd) = hwnd else {
        let _ = ready.send(None);
        return;
    };
    JOBS.with(|j| *j.borrow_mut() = Some(jobs));
    let _ = ready.send(Some(hwnd.0 as isize));
    let mut msg = MSG::default();
    unsafe {
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            DispatchMessageW(&msg);
        }
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_JOBS => {
            let jobs = JOBS.with(|j| j.borrow().clone());
            let mut state = State { hwnd };
            while let Some(job) = jobs.as_ref().and_then(|j| j.try_recv().ok()) {
                state.run(job);
            }
            LRESULT(0)
        }
        // An app is pasting (the clipboard is open on its behalf): hand
        // over the text, and note that it landed.
        WM_RENDERFORMAT if wparam.0 as u32 == u32::from(CF_UNICODETEXT.0) => {
            if let Some((text, reads)) = pending() {
                set_data(u32::from(CF_UNICODETEXT.0), bytes_of(&text));
                let _ = reads.send(());
            }
            LRESULT(0)
        }
        // Bars is closing while its text is still promised: write it out.
        WM_RENDERALLFORMATS => {
            if let Some((text, _)) = pending() {
                if open(hwnd) {
                    if unsafe { GetClipboardOwner() }.ok() == Some(hwnd) {
                        set_data(u32::from(CF_UNICODETEXT.0), bytes_of(&text));
                    }
                    let _ = unsafe { CloseClipboard() };
                }
            }
            LRESULT(0)
        }
        // Someone else took the clipboard.
        WM_DESTROYCLIPBOARD => {
            set_pending(None);
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

fn bytes_of(wide: &[u16]) -> Vec<u8> {
    wide.iter().flat_map(|u| u.to_le_bytes()).collect()
}

/// Opens the clipboard, retrying while another app has it open.
fn open(hwnd: HWND) -> bool {
    for _ in 0..20 {
        if unsafe { OpenClipboard(Some(hwnd)) }.is_ok() {
            return true;
        }
        thread::sleep(Duration::from_millis(10));
    }
    false
}

/// Puts bytes on the (open) clipboard as `format`.
fn set_data(format: u32, bytes: Vec<u8>) {
    unsafe {
        let Ok(mem) = GlobalAlloc(GMEM_MOVEABLE, bytes.len().max(1)) else { return };
        let p = GlobalLock(mem);
        if p.is_null() {
            let _ = GlobalFree(Some(mem));
            return;
        }
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), p as *mut u8, bytes.len());
        let _ = GlobalUnlock(mem);
        // On success the clipboard owns the memory.
        if SetClipboardData(format, Some(HANDLE(mem.0))).is_err() {
            let _ = GlobalFree(Some(mem));
        }
    }
}

fn format_id(name: PCWSTR) -> u32 {
    unsafe { RegisterClipboardFormatW(name) }
}

/// Keeps content out of clipboard history and cloud sync (and, with
/// `monitors`, out of every clipboard manager).
fn hide(monitors: bool) {
    let zero = 0u32.to_le_bytes().to_vec();
    set_data(format_id(w!("CanIncludeInClipboardHistory")), zero.clone());
    set_data(format_id(w!("CanUploadToCloudClipboard")), zero.clone());
    if monitors {
        set_data(format_id(w!("ExcludeClipboardContentFromMonitorProcessing")), zero);
    }
}

/// Formats worth saving: memory, not GDI handles (a bitmap is also there
/// as a DIB), and not the text formats Windows makes from Unicode text.
fn keepable(format: u32, has_unicode: bool) -> bool {
    match format {
        // CF_BITMAP, CF_METAFILEPICT, CF_PALETTE, CF_ENHMETAFILE, and the
        // owner-display and private/GDI-object ranges.
        2 | 3 | 9 | 14 | 0x80 | 0x82 | 0x83 | 0x8E | 0x200..=0x3FF => false,
        // CF_TEXT, CF_OEMTEXT, CF_LOCALE.
        1 | 7 | 16 => !has_unicode,
        _ => true,
    }
}

impl State {
    fn run(&mut self, job: Job) {
        match job {
            Job::Save(reply) => {
                let _ = reply.send(self.save());
            }
            Job::Offer(text, reply) => {
                let _ = reply.send(self.offer(text));
            }
            Job::Restore(saved, reply) => {
                self.restore(saved);
                let _ = reply.send(());
            }
            Job::StillOurs(reply) => {
                let _ = reply.send(unsafe { GetClipboardOwner() }.ok() == Some(self.hwnd));
            }
        }
    }

    fn save(&mut self) -> Option<Saved> {
        if !open(self.hwnd) {
            return None;
        }
        let has_unicode = unsafe { IsClipboardFormatAvailable(u32::from(CF_UNICODETEXT.0)) }.is_ok();
        let mut out = Saved::new();
        let mut format = 0;
        loop {
            format = unsafe { EnumClipboardFormats(format) };
            if format == 0 {
                break;
            }
            if !keepable(format, has_unicode) {
                continue;
            }
            unsafe {
                let Ok(handle) = GetClipboardData(format) else { continue };
                let mem = HGLOBAL(handle.0);
                let size = GlobalSize(mem);
                if size == 0 || size > MAX_FORMAT_BYTES {
                    continue;
                }
                let p = GlobalLock(mem);
                if p.is_null() {
                    continue;
                }
                out.push((format, std::slice::from_raw_parts(p as *const u8, size).to_vec()));
                let _ = GlobalUnlock(mem);
            }
        }
        let _ = unsafe { CloseClipboard() };
        Some(out)
    }

    fn offer(&mut self, text: Vec<u16>) -> anyhow::Result<Receiver<()>> {
        if !open(self.hwnd) {
            anyhow::bail!("another app is holding the clipboard");
        }
        let (tx, rx) = unbounded();
        unsafe {
            let _ = EmptyClipboard();
            // No data: Windows asks this window for it when an app pastes.
            let _ = SetClipboardData(u32::from(CF_UNICODETEXT.0), None);
        }
        hide(true);
        let _ = unsafe { CloseClipboard() };
        set_pending(Some((text, tx)));
        Ok(rx)
    }

    fn restore(&mut self, saved: Saved) {
        if !open(self.hwnd) {
            return;
        }
        unsafe {
            let _ = EmptyClipboard();
        }
        let restored = !saved.is_empty();
        for (format, bytes) in saved {
            set_data(format, bytes);
        }
        // Already in history from when it was first copied.
        if restored {
            hide(false);
        }
        let _ = unsafe { CloseClipboard() };
        set_pending(None);
    }
}

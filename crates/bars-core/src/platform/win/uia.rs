//! Windows UI Automation: the foreground app and window title, and the
//! text around the cursor in the focused field. Calls into other apps time
//! out quickly, so one frozen app can't stall dictation.

use std::cell::OnceCell;

use windows::Win32::Foundation::{CloseHandle, HWND};
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx};
use windows::Win32::System::Threading::{OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW};
use windows::Win32::UI::Accessibility::{
    CUIAutomation, IUIAutomation, IUIAutomation2, IUIAutomationTextPattern, TextPatternRangeEndpoint_End,
    TextPatternRangeEndpoint_Start, TextUnit_Character, UIA_TextPatternId,
};
use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowTextW, GetWindowThreadProcessId};
use windows::core::{Interface, PWSTR};

use crate::platform::focus::Focus;
use crate::text::Surroundings;

/// Milliseconds a call into another app may take.
const TIMEOUT_MS: u32 = 400;

thread_local! {
    // The focus worker thread keeps one client.
    static UIA: OnceCell<Option<IUIAutomation>> = const { OnceCell::new() };
}

fn automation() -> Option<IUIAutomation> {
    UIA.with(|cell| {
        cell.get_or_init(|| unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            let uia: IUIAutomation = CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER).ok()?;
            if let Ok(uia2) = uia.cast::<IUIAutomation2>() {
                let _ = uia2.SetConnectionTimeout(TIMEOUT_MS);
                let _ = uia2.SetTransactionTimeout(TIMEOUT_MS);
            }
            Some(uia)
        })
        .clone()
    })
}

fn field_text(chars: usize) -> windows::core::Result<Option<Surroundings>> {
    let Some(uia) = automation() else { return Ok(None) };
    unsafe {
        let el = uia.GetFocusedElement()?;
        if el.CurrentIsPassword()?.as_bool() {
            return Ok(None);
        }
        let pattern: IUIAutomationTextPattern = el.GetCurrentPatternAs(UIA_TextPatternId)?;
        let selection = pattern.GetSelection()?;
        if selection.Length()? == 0 {
            return Ok(None);
        }
        let range = selection.GetElement(0)?;
        let n = chars as i32;
        // From n characters back up to the selection, and from the
        // selection n characters on.
        let before = range.Clone()?;
        before.MoveEndpointByRange(TextPatternRangeEndpoint_End, &range, TextPatternRangeEndpoint_Start)?;
        before.MoveEndpointByUnit(TextPatternRangeEndpoint_Start, TextUnit_Character, -n)?;
        let after = range.Clone()?;
        after.MoveEndpointByRange(TextPatternRangeEndpoint_Start, &range, TextPatternRangeEndpoint_End)?;
        after.MoveEndpointByUnit(TextPatternRangeEndpoint_End, TextUnit_Character, n)?;
        Ok(Some(Surroundings {
            before: before.GetText(-1)?.to_string(),
            selected: range.GetText(-1)?.to_string(),
            after: after.GetText(-1)?.to_string(),
        }))
    }
}

/// Lowercase executable name without ".exe".
fn exe_name(pid: u32) -> Option<String> {
    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len).is_ok();
        let _ = CloseHandle(process);
        if !ok {
            return None;
        }
        let path = String::from_utf16_lossy(&buf[..len as usize]);
        let file = path.rsplit('\\').next().unwrap_or(&path).to_lowercase();
        Some(file.strip_suffix(".exe").map(str::to_string).unwrap_or(file))
    }
}

fn window_title(hwnd: HWND) -> Option<String> {
    let mut buf = [0u16; 512];
    let len = unsafe { GetWindowTextW(hwnd, &mut buf) };
    (len > 0).then(|| String::from_utf16_lossy(&buf[..len as usize]))
}

pub fn read_focus(field_chars: Option<usize>) -> Focus {
    let hwnd = unsafe { GetForegroundWindow() };
    if hwnd.0.is_null() {
        return Focus::default();
    }
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
    Focus {
        app: (pid != 0).then(|| exe_name(pid)).flatten(),
        name: None,
        pid: (pid != 0).then_some(pid),
        window_title: window_title(hwnd),
        field: field_chars.and_then(|n| field_text(n).ok().flatten()),
        agent: None,
    }
}

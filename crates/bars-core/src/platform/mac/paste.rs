//! Pasting on macOS: the pasteboard as a [`LazyClipboard`] (dictated text
//! is produced only when an app reads it, and hidden from clipboard
//! history apps), and the Cmd+V keystroke itself.

use std::ffi::c_void;
use std::thread;
use std::time::Duration;

use anyhow::anyhow;
use core_foundation::base::{CFType, CFTypeRef, TCFType};
use core_foundation::string::CFStringRef;
use core_graphics::event::{CGEvent, CGEventFlags, CGEventTapLocation, EventField};
use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};
use crossbeam_channel::{Receiver, Sender, unbounded};
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::{AnyThread, DefinedClass, define_class, msg_send};
use objc2_app_kit::{
    NSFileContentsPboardType, NSPasteboard, NSPasteboardItem, NSPasteboardItemDataProvider, NSPasteboardType,
    NSPasteboardTypePDF, NSPasteboardTypeRTFD, NSPasteboardTypeSound, NSPasteboardTypeString, NSPasteboardWriting,
};
use objc2_foundation::{NSArray, NSData, NSObject, NSObjectProtocol, NSString};

use super::hotkeys::SYNTHETIC_MARKER;
use super::main_thread;
use crate::platform::paste::LazyClipboard;

/// Clipboard managers skip items carrying these (nspasteboard.org).
const HIDDEN_MARKERS: &[&str] = &["org.nspasteboard.ConcealedType", "org.nspasteboard.TransientType"];
/// Types larger than this aren't kept (nor restored).
const MAX_TYPE_BYTES: usize = 32 << 20;

pub struct ProviderIvars {
    text: String,
    reads: Sender<()>,
}

define_class!(
    // SAFETY: `NSObject` has no subclassing requirements and this class
    // doesn't implement `Drop`.
    #[unsafe(super(NSObject))]
    #[ivars = ProviderIvars]
    struct TextProvider;

    unsafe impl NSObjectProtocol for TextProvider {}

    unsafe impl NSPasteboardItemDataProvider for TextProvider {
        // An app is pasting: hand over the text, and note that it landed.
        #[unsafe(method(pasteboard:item:provideDataForType:))]
        fn provide(&self, _pasteboard: Option<&NSPasteboard>, item: &NSPasteboardItem, data_type: &NSPasteboardType) {
            let iv = self.ivars();
            let data = NSData::with_bytes(iv.text.as_bytes());
            item.setData_forType(&data, data_type);
            let _ = iv.reads.send(());
        }
    }
);

impl TextProvider {
    fn new(text: String, reads: Sender<()>) -> Retained<Self> {
        let this = Self::alloc().set_ivars(ProviderIvars { text, reads });
        // SAFETY: `NSObject`'s `init` is its designated initializer.
        unsafe { msg_send![super(this), init] }
    }
}

/// Every item on the pasteboard, as (type, data) pairs.
pub type Items = Vec<Vec<(Retained<NSString>, Retained<NSData>)>>;

#[derive(Default)]
pub struct Pasteboard {
    /// The pasteboard's change count right after our offer.
    change_count: Option<isize>,
    /// Kept alive while its offer is on the pasteboard.
    provider: Option<Retained<TextProvider>>,
}

/// Types not kept: file contents, PDF, RTFD and sound are large, and
/// promised files would be written out just by reading them.
fn skipped(t: &NSString) -> bool {
    let fixed = unsafe { [NSFileContentsPboardType, NSPasteboardTypePDF, NSPasteboardTypeRTFD, NSPasteboardTypeSound] };
    fixed.iter().any(|f| *f == t) || {
        let s = t.to_string();
        s == "com.apple.flat-rtfd" || s.starts_with("com.apple.pasteboard.promised-")
    }
}

impl LazyClipboard for Pasteboard {
    type Saved = Items;
    const ASKS_EVERY_PASTE: bool = false;

    fn save(&mut self) -> Option<Items> {
        let pb = NSPasteboard::generalPasteboard();
        let mut out = Items::new();
        for item in pb.pasteboardItems()?.iter() {
            let mut entries = Vec::new();
            for t in item.types().iter() {
                if skipped(&t) {
                    continue;
                }
                if let Some(data) = item.dataForType(&t).filter(|d| d.len() <= MAX_TYPE_BYTES) {
                    entries.push((t, data));
                }
            }
            if !entries.is_empty() {
                out.push(entries);
            }
        }
        Some(out)
    }

    fn offer(&mut self, text: &str) -> anyhow::Result<Receiver<()>> {
        let (tx, rx) = unbounded();
        let text = text.to_string();
        // On the main thread, whose run loop answers apps asking for the data.
        let (provider, count) = main_thread::run(move || {
            let pb = NSPasteboard::generalPasteboard();
            pb.clearContents();
            let provider = TextProvider::new(text, tx);
            let item = NSPasteboardItem::new();
            let types = NSArray::from_slice(&[unsafe { NSPasteboardTypeString }]);
            item.setDataProvider_forTypes(ProtocolObject::from_ref(&*provider), &types);
            for marker in HIDDEN_MARKERS {
                item.setData_forType(&NSData::new(), &NSString::from_str(marker));
            }
            let writer: Retained<ProtocolObject<dyn NSPasteboardWriting>> = ProtocolObject::from_retained(item);
            let ok = pb.writeObjects(&NSArray::from_retained_slice(&[writer]));
            (ok.then_some(provider), pb.changeCount())
        });
        self.provider = Some(provider.ok_or_else(|| anyhow!("couldn't write to the pasteboard"))?);
        self.change_count = Some(count);
        Ok(rx)
    }

    fn still_ours(&mut self) -> bool {
        self.change_count == Some(NSPasteboard::generalPasteboard().changeCount())
    }

    fn restore(&mut self, saved: Items) {
        main_thread::run(move || {
            let pb = NSPasteboard::generalPasteboard();
            pb.clearContents();
            let writers: Vec<Retained<ProtocolObject<dyn NSPasteboardWriting>>> = saved
                .into_iter()
                .map(|entries| {
                    let item = NSPasteboardItem::new();
                    for (t, data) in entries {
                        item.setData_forType(&data, &t);
                    }
                    ProtocolObject::from_retained(item)
                })
                .collect();
            if !writers.is_empty() {
                pb.writeObjects(&NSArray::from_retained_slice(&writers));
            }
        });
        self.provider = None;
        self.change_count = None;
    }
}

#[link(name = "Carbon", kind = "framework")]
unsafe extern "C" {
    fn TISCopyCurrentKeyboardLayoutInputSource() -> CFTypeRef;
    fn TISCopyCurrentASCIICapableKeyboardLayoutInputSource() -> CFTypeRef;
    fn TISGetInputSourceProperty(source: CFTypeRef, key: CFStringRef) -> CFTypeRef;
    static kTISPropertyUnicodeKeyLayoutData: CFStringRef;
    fn LMGetKbdType() -> u8;
    fn UCKeyTranslate(
        layout: *const c_void,
        code: u16,
        action: u16,
        modifiers: u32,
        keyboard_type: u32,
        options: u32,
        dead_keys: *mut u32,
        max_len: usize,
        len: *mut usize,
        chars: *mut u16,
    ) -> i32;
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFDataGetBytePtr(data: CFTypeRef) -> *const c_void;
}

/// kVK_ANSI_V: where "v" is on US layouts.
const KVK_V: u16 = 0x09;

/// The key that types "v" with Command held on the current layout (Dvorak
/// moves it; "Dvorak – QWERTY ⌘" moves it back for shortcuts).
fn paste_key() -> u16 {
    const ACTION_DISPLAY: u16 = 3;
    const NO_DEAD_KEYS: u32 = 1;
    /// cmdKey >> 8, as UCKeyTranslate wants modifiers.
    const COMMAND: u32 = 1;
    // Keyboard-layout lookups abort the app off the main thread.
    main_thread::run(|| unsafe {
        for copy in [TISCopyCurrentKeyboardLayoutInputSource, TISCopyCurrentASCIICapableKeyboardLayoutInputSource] {
            let source = copy();
            if source.is_null() {
                continue;
            }
            let source = CFType::wrap_under_create_rule(source);
            let data = TISGetInputSourceProperty(source.as_CFTypeRef(), kTISPropertyUnicodeKeyLayoutData);
            if data.is_null() {
                continue;
            }
            let layout = CFDataGetBytePtr(data);
            for modifiers in [COMMAND, 0] {
                for code in 0..128u16 {
                    let (mut dead, mut len, mut chars) = (0u32, 0usize, [0u16; 4]);
                    let status = UCKeyTranslate(
                        layout,
                        code,
                        ACTION_DISPLAY,
                        modifiers,
                        LMGetKbdType() as u32,
                        NO_DEAD_KEYS,
                        &mut dead,
                        chars.len(),
                        &mut len,
                        chars.as_mut_ptr(),
                    );
                    if status == 0 && len == 1 && (chars[0] == 'v' as u16 || chars[0] == 'V' as u16) {
                        return Some(code);
                    }
                }
            }
        }
        None
    })
    .unwrap_or(KVK_V)
}

/// Sends Cmd+V as one key with the Command flag on it, instead of pressing
/// Command: keys the user still holds (the end of the shortcut) can't join
/// in, so there's no need to wait for them to be let go.
pub fn press_paste() -> anyhow::Result<()> {
    let code = paste_key();
    let source = CGEventSource::new(CGEventSourceStateID::Private).map_err(|_| anyhow!("no event source"))?;
    for down in [true, false] {
        let event = CGEvent::new_keyboard_event(source.clone(), code, down).map_err(|_| anyhow!("couldn't make a key event"))?;
        event.set_flags(CGEventFlags::CGEventFlagCommand);
        event.set_integer_value_field(EventField::EVENT_SOURCE_USER_DATA, SYNTHETIC_MARKER);
        event.post(CGEventTapLocation::HID);
        if down {
            thread::sleep(Duration::from_millis(5));
        }
    }
    Ok(())
}

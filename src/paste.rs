//! Writing a stored clip back to the pasteboard and synthesizing Cmd+V.

use crate::db::Rep;
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2_app_kit::{NSPasteboard, NSPasteboardItem, NSPasteboardWriting};
use objc2_foundation::{NSArray, NSData, NSString};
use std::ffi::c_void;

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGEventSourceCreate(state: i32) -> *mut c_void;
    fn CGEventCreateKeyboardEvent(src: *mut c_void, key: u16, down: bool) -> *mut c_void;
    fn CGEventSetFlags(ev: *mut c_void, flags: u64);
    fn CGEventPost(tap: u32, ev: *mut c_void);
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFRelease(cf: *const c_void);
}

/// Puts every stored representation back, item by item, exactly as captured.
/// Returns the pasteboard change count after writing.
pub fn write(pb: &NSPasteboard, reps: &[Rep]) -> isize {
    pb.clearContents();
    let mut items: Vec<Retained<ProtocolObject<dyn NSPasteboardWriting>>> = Vec::new();
    let mut cur: Option<(i64, Retained<NSPasteboardItem>)> = None;
    for r in reps {
        if cur.as_ref().map(|(i, _)| *i) != Some(r.item) {
            if let Some((_, it)) = cur.take() {
                items.push(ProtocolObject::from_retained(it));
            }
            cur = Some((r.item, NSPasteboardItem::new()));
        }
        let (_, it) = cur.as_ref().unwrap();
        it.setData_forType(&NSData::with_bytes(&r.data), &NSString::from_str(&r.uti));
    }
    if let Some((_, it)) = cur.take() {
        items.push(ProtocolObject::from_retained(it));
    }
    if !items.is_empty() {
        pb.writeObjects(&NSArray::from_retained_slice(&items));
    }
    pb.changeCount()
}

pub fn plain_text(pb: &NSPasteboard, text: &str) -> isize {
    write(pb, &[Rep { item: 0, uti: "public.utf8-plain-text".into(), data: text.as_bytes().to_vec() }])
}

/// Posts Cmd+V to the frontmost app (needs Accessibility permission).
pub fn post_cmd_v() {
    const KEY_V: u16 = 9;
    const CMD: u64 = 1 << 20;
    unsafe {
        let src = CGEventSourceCreate(1); // kCGEventSourceStateHIDSystemState
        for down in [true, false] {
            let ev = CGEventCreateKeyboardEvent(src, KEY_V, down);
            if !ev.is_null() {
                CGEventSetFlags(ev, CMD);
                CGEventPost(1, ev); // kCGSessionEventTap
                CFRelease(ev);
            }
        }
        if !src.is_null() {
            CFRelease(src);
        }
    }
}

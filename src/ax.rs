//! Accessibility helpers: trust check/prompt and the focused window's title and
//! document path of the app a clip was copied from.

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_foundation::{NSDictionary, NSNumber, NSString, NSURL};
use std::ffi::c_void;

type CFRef = *const c_void;

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXIsProcessTrusted() -> bool;
    fn AXIsProcessTrustedWithOptions(options: CFRef) -> bool;
    fn AXUIElementCreateApplication(pid: i32) -> CFRef;
    fn AXUIElementCopyAttributeValue(el: CFRef, attr: CFRef, value: *mut CFRef) -> i32;
    fn AXUIElementSetMessagingTimeout(el: CFRef, timeout: f32) -> i32;
    static kAXTrustedCheckOptionPrompt: CFRef;
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFRelease(cf: CFRef);
    fn CFGetTypeID(cf: CFRef) -> usize;
    fn CFStringGetTypeID() -> usize;
}

pub fn trusted() -> bool {
    unsafe { AXIsProcessTrusted() }
}

/// Shows the system "allow accessibility" prompt when not trusted yet.
pub fn prompt() -> bool {
    unsafe {
        let key = &*(kAXTrustedCheckOptionPrompt as *const NSString);
        let yes = NSNumber::new_bool(true);
        let dict: Retained<NSDictionary<NSString, AnyObject>> =
            NSDictionary::from_slices(&[key], &[&**yes as &AnyObject]);
        AXIsProcessTrustedWithOptions(&*dict as *const _ as CFRef)
    }
}

unsafe fn copy_attr(el: CFRef, name: &str) -> Option<CFRef> {
    let attr = NSString::from_str(name);
    let mut out: CFRef = std::ptr::null();
    let err = unsafe { AXUIElementCopyAttributeValue(el, &*attr as *const _ as CFRef, &mut out) };
    if err == 0 && !out.is_null() { Some(out) } else { None }
}

unsafe fn string_attr(el: CFRef, name: &str) -> Option<String> {
    unsafe {
        let v = copy_attr(el, name)?;
        let s = if CFGetTypeID(v) == CFStringGetTypeID() {
            Some((*(v as *const NSString)).to_string())
        } else {
            None
        };
        CFRelease(v);
        s.filter(|s| !s.is_empty())
    }
}

/// (window title, document file path) of the focused window of `pid`.
pub fn focused_window_info(pid: i32) -> (Option<String>, Option<String>) {
    if pid <= 0 || !trusted() {
        return (None, None);
    }
    unsafe {
        let app = AXUIElementCreateApplication(pid);
        if app.is_null() {
            return (None, None);
        }
        AXUIElementSetMessagingTimeout(app, 0.25);
        let mut title = None;
        let mut doc = None;
        if let Some(win) = copy_attr(app, "AXFocusedWindow") {
            title = string_attr(win, "AXTitle");
            doc = string_attr(win, "AXDocument").map(|d| {
                NSURL::URLWithString(&NSString::from_str(&d))
                    .and_then(|u| if u.isFileURL() { u.path().map(|p| p.to_string()) } else { None })
                    .unwrap_or(d)
            });
            CFRelease(win);
        }
        CFRelease(app);
        (title, doc)
    }
}

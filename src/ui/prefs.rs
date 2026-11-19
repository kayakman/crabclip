//! Preferences window, built in code. Every control applies immediately.

use super::app;
use crate::settings::{UI_SCALES, ui_scale_stop};
use crate::{log, timefmt};
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{MainThreadMarker, MainThreadOnly, msg_send, sel};
use objc2_app_kit::{
    NSApplication, NSBackingStoreType, NSButton, NSColor, NSControlStateValueOff,
    NSControlStateValueOn, NSFont, NSPopUpButton, NSSlider, NSTextAlignment, NSTextField, NSView, NSWindow,
    NSWindowStyleMask, NSWorkspace,
};
use objc2_foundation::{NSArray, NSNotification, NSPoint, NSRect, NSSize, NSString, NSURL};
use std::cell::RefCell;

const OPEN_AT: &[(&str, &str)] = &[
    ("last", "Last window position"),
    ("menubar", "Under the menu bar icon"),
    ("mouse", "At the mouse pointer"),
];
const ROWS: &[(f64, &str)] = &[(52.0, "Compact"), (72.0, "Normal"), (96.0, "Large"), (128.0, "Huge")];
const FONTS: &[f64] = &[11.0, 12.0, 13.0, 14.0, 15.0, 16.0, 18.0, 20.0, 24.0];
const ZSTD: &[(i32, &str)] = &[(3, "Fast (zstd 3)"), (9, "Balanced (zstd 9)"), (19, "Smallest (zstd 19)")];
const KEEP: &[(u32, &str)] = &[(0, "Forever"), (365, "1 year"), (180, "6 months"), (90, "90 days"), (30, "30 days")];

struct Prefs {
    window: Retained<NSWindow>,
    hotkey: Retained<NSTextField>,
    open_at: Retained<NSPopUpButton>,
    login: Retained<NSButton>,
    move_top: Retained<NSButton>,
    enter_pastes: Retained<NSButton>,
    ocr: Retained<NSButton>,
    rows: Retained<NSPopUpButton>,
    font: Retained<NSPopUpButton>,
    scale: Retained<NSSlider>,
    scale_label: Retained<NSTextField>,
    img_pct: Retained<NSTextField>,
    img_mp: Retained<NSTextField>,
    zstd: Retained<NSPopUpButton>,
    keep: Retained<NSPopUpButton>,
    ignore: Retained<NSTextField>,
    stats: Retained<NSTextField>,
    ax: Retained<NSTextField>,
}

thread_local! {
    static PREFS: RefCell<Option<Prefs>> = const { RefCell::new(None) };
}

fn ns(s: &str) -> Retained<NSString> {
    NSString::from_str(s)
}

fn rect(x: f64, y: f64, w: f64, h: f64) -> NSRect {
    NSRect::new(NSPoint::new(x, y), NSSize::new(w, h))
}

struct Form<'a> {
    mtm: MainThreadMarker,
    view: &'a NSView,
    y: f64,
    target: &'a AnyObject,
}

impl Form<'_> {
    fn label(&mut self, text: &str) {
        let l = NSTextField::labelWithString(&ns(text), self.mtm);
        l.setFrame(rect(16.0, self.y + 3.0, 196.0, 18.0));
        l.setAlignment(NSTextAlignment::Right);
        self.view.addSubview(&l);
    }

    fn note(&mut self, text: &str) -> Retained<NSTextField> {
        let l = NSTextField::wrappingLabelWithString(&ns(text), self.mtm);
        l.setFrame(rect(222.0, self.y - 2.0, 330.0, 30.0));
        l.setFont(Some(&NSFont::systemFontOfSize(11.0)));
        l.setTextColor(Some(&NSColor::secondaryLabelColor()));
        self.view.addSubview(&l);
        self.y -= 30.0;
        l
    }

    fn popup(&mut self, text: &str, items: &[&str], selected: usize) -> Retained<NSPopUpButton> {
        self.label(text);
        let p = NSPopUpButton::initWithFrame_pullsDown(NSPopUpButton::alloc(self.mtm), rect(220.0, self.y, 230.0, 26.0), false);
        for i in items {
            p.addItemWithTitle(&ns(i));
        }
        p.selectItemAtIndex(selected as isize);
        unsafe {
            p.setTarget(Some(self.target));
            p.setAction(Some(sel!(prefChanged:)));
        }
        self.view.addSubview(&p);
        self.y -= 34.0;
        p
    }

    fn check(&mut self, text: &str, on: bool) -> Retained<NSButton> {
        let b = unsafe {
            NSButton::checkboxWithTitle_target_action(&ns(text), Some(self.target), Some(sel!(prefChanged:)), self.mtm)
        };
        b.setFrame(rect(220.0, self.y, 330.0, 22.0));
        b.setState(if on { NSControlStateValueOn } else { NSControlStateValueOff });
        self.view.addSubview(&b);
        self.y -= 28.0;
        b
    }

    fn field(&mut self, x: f64, w: f64, value: &str) -> Retained<NSTextField> {
        let f = NSTextField::textFieldWithString(&ns(value), self.mtm);
        f.setFrame(rect(x, self.y, w, 24.0));
        unsafe {
            f.setTarget(Some(self.target));
            f.setAction(Some(sel!(prefChanged:)));
        }
        self.view.addSubview(&f);
        f
    }

    fn button(&mut self, x: f64, w: f64, title: &str, tag: isize) -> Retained<NSButton> {
        let b = unsafe { NSButton::buttonWithTitle_target_action(&ns(title), Some(self.target), Some(sel!(prefAction:)), self.mtm) };
        b.setFrame(rect(x, self.y - 3.0, w, 30.0));
        b.setTag(tag);
        self.view.addSubview(&b);
        b
    }

    fn inline_label(&mut self, x: f64, w: f64, text: &str) {
        let l = NSTextField::labelWithString(&ns(text), self.mtm);
        l.setFrame(rect(x, self.y + 3.0, w, 18.0));
        self.view.addSubview(&l);
    }
}

pub fn show() {
    let a = app();
    let mtm = a.mtm;
    let existing = PREFS.with(|p| p.borrow().as_ref().map(|p| p.window.clone()));
    if let Some(w) = existing {
        // ⌘, again closes them; the gear / menu bar item only brings them up.
        let by_key = NSApplication::sharedApplication(mtm)
            .currentEvent()
            .is_some_and(|e| e.r#type() == objc2_app_kit::NSEventType::KeyDown);
        if by_key && w.isVisible() && w.isKeyWindow() {
            w.close(); // ⌘, again closes them (and brings the history back)
            return;
        }
        refresh_status();
        a.remember_front_app();
        a.hide_for_window();
        bring_forward(&w);
        return;
    }
    let s = a.settings.borrow().clone();
    let (w, h) = (580.0, 788.0);
    let window = unsafe {
        NSWindow::initWithContentRect_styleMask_backing_defer(
            NSWindow::alloc(mtm),
            rect(0.0, 0.0, w, h),
            NSWindowStyleMask::Titled | NSWindowStyleMask::Closable,
            NSBackingStoreType::Buffered,
            false,
        )
    };
    unsafe { window.setReleasedWhenClosed(false) };
    window.setTitle(&ns("crabclip Preferences"));
    let view = NSView::initWithFrame(NSView::alloc(mtm), rect(0.0, 0.0, w, h));
    let target: &AnyObject = &a.handler;
    let mut f = Form { mtm, view: &view, y: h - 44.0, target };

    f.label("Show history with:");
    let hotkey = f.field(220.0, 150.0, &s.hotkey);
    unsafe {
        hotkey.setAction(Some(sel!(prefAction:)));
    }
    hotkey.setTag(101);
    f.button(376.0, 80.0, "Set", 101);
    f.y -= 30.0;
    f.note("e.g. cmd+alt+v, ctrl+shift+space, cmd+shift+KeyC");

    let open_idx = OPEN_AT.iter().position(|(k, _)| *k == s.open_at).unwrap_or(0);
    let open_at = f.popup("Open window at:", &OPEN_AT.iter().map(|(_, t)| *t).collect::<Vec<_>>(), open_idx);
    let row_idx = ROWS.iter().position(|(v, _)| (*v - s.row_height).abs() < 1.0).unwrap_or(1);
    let rows = f.popup("Row size:", &ROWS.iter().map(|(_, t)| *t).collect::<Vec<_>>(), row_idx);
    let font_idx = FONTS.iter().position(|v| (*v - s.font_size).abs() < 0.5).unwrap_or(2);
    let font_titles: Vec<String> = FONTS.iter().map(|v| if *v == 13.0 { format!("{v} pt (default)") } else { format!("{v} pt") }).collect();
    let font = f.popup("Text size:", &font_titles.iter().map(|t| t.as_str()).collect::<Vec<_>>(), font_idx);

    f.label("Interface size:");
    let scale = unsafe {
        NSSlider::sliderWithValue_minValue_maxValue_target_action(
            ui_scale_stop(s.ui_scale) as f64,
            0.0,
            (UI_SCALES.len() - 1) as f64,
            Some(target),
            Some(sel!(prefChanged:)),
            mtm,
        )
    };
    scale.setFrame(rect(220.0, f.y, 230.0, 24.0));
    scale.setNumberOfTickMarks(UI_SCALES.len() as isize);
    scale.setAllowsTickMarkValuesOnly(true);
    scale.setContinuous(false);
    view.addSubview(&scale);
    let scale_label = NSTextField::labelWithString(&ns(&format!("{}%", s.ui_scale)), mtm);
    scale_label.setFrame(rect(458.0, f.y + 3.0, 60.0, 18.0));
    view.addSubview(&scale_label);
    f.y -= 30.0;
    f.note("Zooms the whole window: text, rows, icons and its size.");

    f.label("General:");
    let login = f.check("Launch at login (restarts if it ever crashes)", s.launch_at_login);
    let move_top = f.check("Move pasted clips to the top", s.move_to_top);
    let enter_pastes = f.check("⏎ pastes into the app in front", s.enter_pastes);
    f.note("Off: ⏎ only copies the clip. ⌘⏎ always does the other.");
    let ocr = f.check("Recognize text in images (searchable)", s.ocr);
    f.y -= 6.0;

    f.label("Store images larger than:");
    let img_mp = f.field(220.0, 50.0, &format!("{}", s.img_scale_min_mp));
    f.inline_label(276.0, 60.0, "MP at");
    let img_pct = f.field(322.0, 50.0, &format!("{}", s.img_scale_pct));
    f.inline_label(378.0, 140.0, "% of their size");
    f.y -= 30.0;
    f.note("100% keeps every image byte-for-byte (default). Smaller images are always kept as-is; pasting uses what is stored.");

    let zstd_idx = ZSTD.iter().position(|(v, _)| *v == s.zstd_level).unwrap_or(1);
    let zstd = f.popup("Compression:", &ZSTD.iter().map(|(_, t)| *t).collect::<Vec<_>>(), zstd_idx);
    let keep_idx = KEEP.iter().position(|(v, _)| *v == s.keep_days).unwrap_or(0);
    let keep = f.popup("Keep history:", &KEEP.iter().map(|(_, t)| *t).collect::<Vec<_>>(), keep_idx);
    f.note("Starred clips are always kept.");

    f.label("Ignore apps (bundle ids):");
    let ignore = f.field(220.0, 330.0, &s.ignore_apps.join(", "));
    f.y -= 30.0;
    f.note("Password-manager copies marked concealed are never recorded.");

    f.label("Database:");
    let stats = NSTextField::labelWithString(&ns(""), mtm);
    stats.setFrame(rect(220.0, f.y + 3.0, 340.0, 18.0));
    view.addSubview(&stats);
    f.y -= 30.0;
    f.button(216.0, 150.0, "Reveal in Finder", 102);
    f.button(370.0, 190.0, "Import Collective history", 103);
    f.y -= 40.0;

    f.label("Accessibility:");
    let ax = NSTextField::labelWithString(&ns(""), mtm);
    ax.setFrame(rect(220.0, f.y + 3.0, 200.0, 18.0));
    view.addSubview(&ax);
    f.button(400.0, 160.0, "Open Settings…", 104);
    f.y -= 30.0;
    f.note("Needed to paste into the frontmost app (⏎) and to record window titles and documents.");

    window.setContentView(Some(&view));
    window.center();
    PREFS.with(|p| {
        *p.borrow_mut() = Some(Prefs {
            window: window.clone(),
            hotkey,
            open_at,
            login,
            move_top,
            enter_pastes,
            ocr,
            rows,
            font,
            scale,
            scale_label,
            img_pct,
            img_mp,
            zstd,
            keep,
            ignore,
            stats,
            ax,
        })
    });
    window.setDelegate(Some(objc2::runtime::ProtocolObject::from_ref(&*a.handler)));
    refresh_status();
    a.remember_front_app();
    a.hide_for_window();
    bring_forward(&window);
}

/// Front and key even though crabclip is an accessory app that is usually not
/// active (the history panel never activates it).
fn bring_forward(w: &NSWindow) {
    let nsapp = NSApplication::sharedApplication(app().mtm);
    #[allow(deprecated)]
    nsapp.activateIgnoringOtherApps(true);
    w.makeKeyAndOrderFront(None);
    w.orderFrontRegardless();
}

/// Preferences is closing (button, ⌘W or ⌘, again): back to the history.
pub fn closing(n: &NSNotification) {
    let is_prefs = PREFS.with(|p| {
        p.borrow().as_ref().is_some_and(|p| {
            n.object().is_some_and(|o| std::ptr::eq(&*o as *const AnyObject, &*p.window as *const NSWindow as *const AnyObject))
        })
    });
    if is_prefs {
        dispatch2::DispatchQueue::main().exec_async(|| {
            let a = app();
            if a.ui().panel.isVisible() {
                // Pinned and still up: keep its search and filter.
                a.ui().panel.makeKeyAndOrderFront(None);
            } else {
                a.show();
            }
        });
    }
}

fn refresh_status() {
    let a = app();
    PREFS.with(|p| {
        let p = p.borrow();
        let Some(p) = p.as_ref() else { return };
        p.stats.setStringValue(&ns(&format!(
            "{} clips · {} on disk",
            a.db.count().unwrap_or(0),
            timefmt::bytes(a.db.file_size())
        )));
        let ok = crate::ax::trusted();
        p.ax.setStringValue(&ns(if ok { "✓ Allowed" } else { "✗ Not allowed yet" }));
    });
}

/// The interface size changed elsewhere (⌘+ / ⌘-): move the slider along.
pub fn sync_scale() {
    let pct = app().settings.borrow().ui_scale;
    PREFS.with(|p| {
        if let Some(p) = p.borrow().as_ref() {
            p.scale.setDoubleValue(ui_scale_stop(pct) as f64);
            p.scale_label.setStringValue(&ns(&format!("{pct}%")));
        }
    });
}

fn on(b: &NSButton) -> bool {
    b.state() == NSControlStateValueOn
}

/// Any control changed: read the whole form back into the settings.
pub fn changed(_sender: Option<&AnyObject>) {
    let a = app();
    let (old, new) = PREFS.with(|p| {
        let p = p.borrow();
        let p = p.as_ref()?;
        let old = a.settings.borrow().clone();
        let mut s = old.clone();
        s.open_at = OPEN_AT.get(p.open_at.indexOfSelectedItem().max(0) as usize).map(|x| x.0).unwrap_or("last").into();
        s.row_height = ROWS.get(p.rows.indexOfSelectedItem().max(0) as usize).map(|x| x.0).unwrap_or(72.0);
        s.font_size = FONTS.get(p.font.indexOfSelectedItem().max(0) as usize).copied().unwrap_or(13.0);
        let stop = (p.scale.doubleValue().round().max(0.0) as usize).min(UI_SCALES.len() - 1);
        // Leave a size set outside the stops alone until the slider moves off it.
        if stop != ui_scale_stop(old.ui_scale) {
            s.ui_scale = UI_SCALES[stop];
        }
        p.scale_label.setStringValue(&ns(&format!("{}%", s.ui_scale)));
        s.launch_at_login = on(&p.login);
        s.move_to_top = on(&p.move_top);
        s.enter_pastes = on(&p.enter_pastes);
        s.ocr = on(&p.ocr);
        s.img_scale_pct = p.img_pct.stringValue().to_string().trim().parse::<u32>().unwrap_or(100).clamp(5, 100);
        s.img_scale_min_mp = p.img_mp.stringValue().to_string().trim().parse::<f64>().unwrap_or(12.0).max(0.0);
        p.img_pct.setStringValue(&ns(&s.img_scale_pct.to_string()));
        s.zstd_level = ZSTD.get(p.zstd.indexOfSelectedItem().max(0) as usize).map(|x| x.0).unwrap_or(9);
        s.keep_days = KEEP.get(p.keep.indexOfSelectedItem().max(0) as usize).map(|x| x.0).unwrap_or(0);
        s.ignore_apps = p
            .ignore
            .stringValue()
            .to_string()
            .split(',')
            .map(|x| x.trim().to_string())
            .filter(|x| !x.is_empty())
            .collect();
        Some((old, s))
    })
    .unwrap_or_else(|| {
        let s = a.settings.borrow().clone();
        (s.clone(), s)
    });
    if old == new {
        return;
    }
    if new.keep_days > 0 && new.keep_days != old.keep_days && (old.keep_days == 0 || new.keep_days < old.keep_days) {
        let before = timefmt::now_ms() - new.keep_days as i64 * 86_400_000;
        let n: i64 = a
            .db
            .conn
            .query_row("SELECT COUNT(*) FROM clips WHERE starred=0 AND copied_at < ?", [before], |r| r.get(0))
            .unwrap_or(0);
        if n > 0
            && !a.confirm(
                &format!("Delete {n} clips older than {} days?", new.keep_days),
                "Unstarred clips older than that are removed now and from then on. Starred clips are kept.",
                "Delete",
            )
        {
            PREFS.with(|p| {
                if let Some(p) = p.borrow().as_ref() {
                    let idx = KEEP.iter().position(|(v, _)| *v == old.keep_days).unwrap_or(0);
                    p.keep.selectItemAtIndex(idx as isize);
                }
            });
            return;
        }
    }
    new.save(&a.db);
    *a.settings.borrow_mut() = new.clone();
    log!("prefs: saved");
    if old.launch_at_login != new.launch_at_login {
        if !new.launch_at_login && super::running_as_agent() {
            // Unregistering the agent would kill this very process: start a
            // plain copy that takes over and unregisters it on launch.
            let bundle = objc2_foundation::NSBundle::mainBundle().bundlePath().to_string();
            let _ = std::process::Command::new("/usr/bin/open").args(["-n", &bundle, "--args", "--takeover"]).spawn();
        } else if let Err(e) = crate::login::set_enabled(new.launch_at_login) {
            log!("login item: {e}");
        }
    }
    if old.row_height != new.row_height {
        a.apply_buttons();
        a.invalidate_all();
        a.ui().table.reloadData();
    }
    if old.font_size != new.font_size {
        a.ui().table.reloadData();
        a.update_preview();
    }
    if old.ui_scale != new.ui_scale {
        a.rescale_window(old.ui_scale);
    }
    if old.keep_days != new.keep_days {
        let _ = a.jobs.send(crate::worker::Job::Prune);
    }
}

pub fn action(sender: Option<&AnyObject>) {
    let a = app();
    let tag: isize = sender.map(|s| unsafe { msg_send![s, tag] }).unwrap_or(0);
    match tag {
        101 => {
            let spec = PREFS.with(|p| p.borrow().as_ref().map(|p| p.hotkey.stringValue().to_string()));
            let Some(spec) = spec else { return };
            let old = a.settings.borrow().hotkey.clone();
            a.settings.borrow_mut().hotkey = spec.trim().to_string();
            match a.register_hotkey() {
                Ok(()) => a.settings.borrow().save(&a.db),
                Err(e) => {
                    a.settings.borrow_mut().hotkey = old.clone();
                    let _ = a.register_hotkey();
                    PREFS.with(|p| {
                        if let Some(p) = p.borrow().as_ref() {
                            p.hotkey.setStringValue(&ns(&old));
                        }
                    });
                    a.confirm("Cannot use that shortcut", &e, "OK");
                }
            }
        }
        102 => {
            let url = NSURL::fileURLWithPath(&ns(&crate::db::Db::default_path().display().to_string()));
            NSWorkspace::sharedWorkspace().activateFileViewerSelectingURLs(&NSArray::from_retained_slice(&[url]));
        }
        103 => import_collective(),
        104 => {
            crate::ax::prompt();
            if let Some(u) = NSURL::URLWithString(&ns(
                "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility",
            )) {
                NSWorkspace::sharedWorkspace().openURL(&u);
            }
        }
        _ => {}
    }
}

/// Runs the importer as a child process (it loads the whole archive in memory,
/// which is then given back to the system when it exits).
fn import_collective() {
    let paths = crate::collective::default_paths();
    if paths.is_empty() {
        app().confirm("No Collective history found", "Collective's archive was not found in its container.", "OK");
        return;
    }
    if !app().confirm(
        "Import Collective history?",
        "Collective saves its history when it quits: quit Collective first to include its latest clips. Clips already in crabclip are merged, not duplicated.",
        "Import",
    ) {
        return;
    }
    let exe = std::env::current_exe().unwrap_or_default();
    std::thread::spawn(move || {
        let out = std::process::Command::new(exe).arg("--import-collective").output();
        let msg = match out {
            Ok(o) => String::from_utf8_lossy(&o.stdout).trim().to_string(),
            Err(e) => format!("failed: {e}"),
        };
        dispatch2::DispatchQueue::main().exec_async(move || {
            let a = app();
            a.invalidate_all();
            a.run_search(true);
            refresh_status();
            a.confirm("Collective import finished", &msg, "OK");
        });
    });
}

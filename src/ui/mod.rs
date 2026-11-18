//! The menu bar app: status item, global hotkey, pasteboard watcher, the
//! history panel (list + preview) and the paste flow.

mod build;
pub mod classes;
mod draw;
pub mod prefs;

use crate::db::{Db, Detail, Filter, Kind, RowInfo};
use crate::ingest::{Capture, SKIP_MARKERS};
use crate::settings::Settings;
use crate::worker::{Event, Job};
use crate::{log, timefmt};
use classes::{ClipPanel, ClipTable, Handler, RowView};
use dispatch2::DispatchQueue;
use global_hotkey::hotkey::HotKey;
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject, Sel};
use objc2::{AnyThread, DefinedClass, MainThreadMarker, Message, msg_send, sel};
use objc2_app_kit::{
    NSAlert, NSAlertFirstButtonReturn, NSApplication, NSApplicationActivationOptions,
    NSApplicationActivationPolicy, NSAttributedStringAppKitDocumentFormats, NSButton, NSColor,
    NSControl, NSEvent, NSEventModifierFlags, NSEventType, NSFont, NSImage, NSImageView,
    NSMenu, NSPasteboard, NSPopUpButton, NSRunningApplication, NSScreen, NSScrollView,
    NSSearchField, NSStatusItem, NSTableView, NSTextField, NSTextView, NSUserInterfaceItemIdentification, NSView, NSWorkspace,
};
use objc2_foundation::{
    NSAttributedString, NSBundle, NSData, NSDistributedNotificationCenter, NSIndexSet,
    NSNotification, NSPoint, NSRange, NSRect, NSRunLoop, NSRunLoopCommonModes, NSString,
    NSTimer, NSUserDefaults,
};
use std::cell::{Cell, OnceCell, RefCell};
use std::collections::{HashMap, HashSet, VecDeque};
use std::rc::Rc;
use std::sync::mpsc::Sender;
use std::time::Instant;

pub const SHOW_NOTIFICATION: &str = "com.rod.cliprust.show";
pub const QUIT_NOTIFICATION: &str = "com.rod.cliprust.quit";
pub const AGENT_LABEL: &str = "com.rod.cliprust.agent";
pub const PANEL_AUTOSAVE: &str = "cliprust.panel";
const CACHE_ROWS: usize = 160;
const SEARCH_LIMIT: usize = 200_000;

thread_local! {
    static APP: OnceCell<&'static App> = const { OnceCell::new() };
}

pub fn app() -> &'static App {
    APP.with(|a| *a.get().expect("app not started"))
}

pub struct RowData {
    pub info: RowInfo,
    pub thumb: Option<Retained<NSImage>>,
}

pub struct Ui {
    pub panel: Retained<ClipPanel>,
    pub table: Retained<ClipTable>,
    pub search: Retained<NSSearchField>,
    pub app_popup: Retained<NSPopUpButton>,
    pub star_btn: Retained<NSButton>,
    pub pin_btn: Retained<NSButton>,
    pub date_label: Retained<NSTextField>,
    pub count_label: Retained<NSTextField>,
    pub text_scroll: Retained<NSScrollView>,
    pub text_view: Retained<NSTextView>,
    pub image_view: Retained<NSImageView>,
    pub meta: Retained<NSTextField>,
    pub status_item: Retained<NSStatusItem>,
    pub status_menu: Retained<NSMenu>,
}

pub struct App {
    pub mtm: MainThreadMarker,
    pub handler: Retained<Handler>,
    pub db: Db,
    pub settings: RefCell<Settings>,
    pub jobs: Sender<Job>,
    pub ids: RefCell<Vec<i64>>,
    cache: RefCell<HashMap<i64, Rc<RowData>>>,
    cache_order: RefCell<VecDeque<i64>>,
    icons: RefCell<HashMap<String, Retained<NSImage>>>,
    pub ui: OnceCell<Ui>,
    last_change: Cell<isize>,
    prev_app: RefCell<Option<Retained<NSRunningApplication>>>,
    app_filter: RefCell<Option<String>>,
    hotkey: RefCell<Option<(GlobalHotKeyManager, Option<HotKey>)>>,
    pasting: Cell<bool>,
    last_hide: Cell<Option<Instant>>,
    last_prune: Cell<Option<Instant>>,
    seen_apps: RefCell<HashSet<String>>,
    dirty: Cell<bool>,
    own_bundle: Option<String>,
}

/// Entry point: runs the Cocoa app (never returns).
pub fn run() {
    let mtm = MainThreadMarker::new().expect("must run on the main thread");
    let nsapp = NSApplication::sharedApplication(mtm);
    nsapp.setActivationPolicy(NSApplicationActivationPolicy::Accessory);

    // Single instance. The login agent's copy (restarted by launchd if it ever
    // crashes) takes over from a hand-launched one; any other second launch just
    // asks the running copy to show its window.
    if let Some(bid) = NSBundle::mainBundle().bundleIdentifier() {
        let me = std::process::id() as i32;
        let others = || {
            NSRunningApplication::runningApplicationsWithBundleIdentifier(&bid)
                .iter()
                .filter(|r| r.processIdentifier() != me)
                .count()
        };
        if others() > 0 {
            let is_agent = std::env::var("XPC_SERVICE_NAME").is_ok_and(|v| v == AGENT_LABEL);
            let name = if is_agent { QUIT_NOTIFICATION } else { SHOW_NOTIFICATION };
            post_distributed(name);
            if !is_agent {
                std::process::exit(0);
            }
            for _ in 0..50 {
                std::thread::sleep(std::time::Duration::from_millis(100));
                if others() == 0 {
                    break;
                }
            }
        }
    }

    let handler = Handler::new(mtm);
    nsapp.setDelegate(Some(ProtocolObject::from_ref(&*handler)));
    nsapp.run();
    drop(handler);
}

fn start(handler: &Handler) {
    let mtm = MainThreadMarker::new().unwrap();
    log!("cliprust {} starting (pid {})", env!("CARGO_PKG_VERSION"), std::process::id());
    let db_path = Db::default_path();
    let db = match Db::open(&db_path) {
        Ok(db) => db,
        Err(e) => {
            log!("fatal: cannot open {}: {e}", db_path.display());
            std::process::exit(2);
        }
    };
    let settings = Settings::load(&db);
    let notify: crate::worker::Notify = std::sync::Arc::new(|e: Event| {
        DispatchQueue::main().exec_async(move || app().on_event(e));
    });
    let (jobs, _ocr_wake) = crate::worker::spawn(db_path, notify);
    let pb = NSPasteboard::generalPasteboard();
    let app_state = App {
        mtm,
        handler: handler.retain(),
        db,
        settings: RefCell::new(settings),
        jobs,
        ids: RefCell::new(Vec::new()),
        cache: RefCell::new(HashMap::new()),
        cache_order: RefCell::new(VecDeque::new()),
        icons: RefCell::new(HashMap::new()),
        ui: OnceCell::new(),
        last_change: Cell::new(pb.changeCount()),
        prev_app: RefCell::new(None),
        app_filter: RefCell::new(None),
        hotkey: RefCell::new(None),
        pasting: Cell::new(false),
        last_hide: Cell::new(None),
        last_prune: Cell::new(None),
        seen_apps: RefCell::new(HashSet::new()),
        dirty: Cell::new(true),
        own_bundle: NSBundle::mainBundle().bundleIdentifier().map(|s| s.to_string()),
    };
    let a: &'static App = Box::leak(Box::new(app_state));
    APP.with(|cell| {
        let _ = cell.set(a);
    });

    build::main_menu(a);
    let ui = build::build(a);
    let _ = a.ui.set(ui);
    a.apply_buttons();

    if let Ok(out) = std::env::var("CLIPRUST_SNAPSHOT") {
        a.snapshot(&out);
        std::process::exit(0);
    }

    if let Err(e) = a.register_hotkey() {
        log!("hotkey: {e}");
    }

    unsafe {
        let timer = NSTimer::timerWithTimeInterval_target_selector_userInfo_repeats(
            0.25,
            &a.handler,
            sel!(tick:),
            None,
            true,
        );
        NSRunLoop::mainRunLoop().addTimer_forMode(&timer, NSRunLoopCommonModes);
        let center = NSDistributedNotificationCenter::defaultCenter();
        center.addObserver_selector_name_object(
            &a.handler,
            sel!(showFromNotification:),
            Some(&NSString::from_str(SHOW_NOTIFICATION)),
            None,
        );
        center.addObserver_selector_name_object(
            &a.handler,
            sel!(quitFromNotification:),
            Some(&NSString::from_str(QUIT_NOTIFICATION)),
            None,
        );
    }

    let installed = NSBundle::mainBundle().bundlePath().to_string().starts_with("/Applications/");
    if a.settings.borrow().launch_at_login && installed && !crate::login::is_enabled() {
        if let Err(e) = crate::login::set_enabled(true) {
            log!("login item: {e}");
        }
    }
    if !crate::ax::trusted() && a.db.get_setting("ax_prompted").is_none() {
        let _ = a.db.set_setting("ax_prompted", "1");
        crate::ax::prompt();
    }
    let _ = a.jobs.send(Job::Prune);
    a.last_prune.set(Some(Instant::now()));
    log!("ready: {} clips", a.db.count().unwrap_or(0));
}

impl App {
    pub fn ui(&self) -> &Ui {
        self.ui.get().expect("ui")
    }

    /// Debug aid: render the panel offscreen to a PNG (never shown, never key).
    /// CLIPRUST_SNAPSHOT_QUERY / _ROW pick what is displayed.
    fn snapshot(&self, out: &str) {
        let ui = self.ui();
        self.rebuild_app_popup();
        let q = std::env::var("CLIPRUST_SNAPSHOT_QUERY").unwrap_or_default();
        ui.search.setStringValue(&NSString::from_str(&q));
        self.run_search(true);
        let row: isize = std::env::var("CLIPRUST_SNAPSHOT_ROW").ok().and_then(|r| r.parse().ok()).unwrap_or(0);
        self.select_row(row);
        self.update_preview();
        let Some(view) = ui.panel.contentView() else { return };
        view.layoutSubtreeIfNeeded();
        ui.table.tile();
        let b = view.bounds();
        let Some(rep) = view.bitmapImageRepForCachingDisplayInRect(b) else { return };
        view.cacheDisplayInRect_toBitmapImageRep(b, &rep);
        let props = objc2_foundation::NSDictionary::new();
        if let Some(png) = unsafe { rep.representationUsingType_properties(objc2_app_kit::NSBitmapImageFileType::PNG, &props) } {
            let _ = std::fs::write(out, png.to_vec());
            log!("snapshot written to {out}");
        }
    }

    // ---------------------------------------------------------------- hotkey

    pub fn register_hotkey(&self) -> Result<(), String> {
        let spec = self.settings.borrow().hotkey.clone();
        let hk: HotKey = spec.parse().map_err(|e| format!("bad shortcut {spec:?}: {e}"))?;
        let mut slot = self.hotkey.borrow_mut();
        if slot.is_none() {
            let m = GlobalHotKeyManager::new().map_err(|e| e.to_string())?;
            GlobalHotKeyEvent::set_event_handler(Some(|e: GlobalHotKeyEvent| {
                if e.state == HotKeyState::Pressed {
                    DispatchQueue::main().exec_async(|| app().hotkey_pressed());
                }
            }));
            *slot = Some((m, None));
        }
        let (m, current) = slot.as_mut().unwrap();
        if let Some(old) = current.take() {
            let _ = m.unregister(old);
        }
        m.register(hk).map_err(|e| format!("cannot register {spec}: {e}"))?;
        *current = Some(hk);
        log!("hotkey: {spec}");
        Ok(())
    }

    fn hotkey_pressed(&self) {
        let p = &self.ui().panel;
        if p.isVisible() && p.isKeyWindow() {
            self.hide();
        } else {
            self.show();
        }
    }

    pub fn status_clicked(&self) {
        let nsapp = NSApplication::sharedApplication(self.mtm);
        let ev = nsapp.currentEvent();
        let right = ev.as_ref().is_some_and(|e| {
            e.r#type() == NSEventType::RightMouseUp
                || e.modifierFlags().contains(NSEventModifierFlags::Control)
        });
        let ui = self.ui();
        if right {
            ui.status_item.setMenu(Some(&ui.status_menu));
            if let Some(b) = ui.status_item.button(self.mtm) {
                unsafe { b.performClick(None) };
            }
            ui.status_item.setMenu(None);
            return;
        }
        let just_hidden = self.last_hide.get().is_some_and(|t| t.elapsed().as_millis() < 300);
        if ui.panel.isVisible() || just_hidden {
            self.hide();
        } else {
            self.show();
        }
    }

    // --------------------------------------------------------------- capture

    pub fn tick(&self) {
        let pb = NSPasteboard::generalPasteboard();
        let cc = pb.changeCount();
        if cc != self.last_change.get() {
            self.last_change.set(cc);
            self.capture(&pb);
        }
        if self.last_prune.get().is_none_or(|t| t.elapsed().as_secs() > 3600) {
            self.last_prune.set(Some(Instant::now()));
            let _ = self.jobs.send(Job::Prune);
        }
    }

    fn capture(&self, pb: &NSPasteboard) {
        let Some(items) = pb.pasteboardItems() else { return };
        let mut reps = Vec::new();
        for (i, item) in items.iter().enumerate() {
            for t in item.types().iter() {
                let uti = t.to_string();
                if SKIP_MARKERS.contains(&uti.as_str()) {
                    log!("capture: skipped concealed/transient clip ({uti})");
                    return;
                }
                if let Some(d) = item.dataForType(&t) {
                    if d.len() <= 512 * 1024 * 1024 {
                        reps.push(crate::db::Rep { item: i as i64, uti, data: d.to_vec() });
                    }
                }
            }
        }
        if reps.is_empty() {
            return;
        }
        let ws = NSWorkspace::sharedWorkspace();
        let front = ws.frontmostApplication();
        let mut info = crate::db::AppInfo::default();
        let mut pid = 0;
        if let Some(f) = &front {
            info.bundle = f.bundleIdentifier().map(|s| s.to_string());
            info.name = f.localizedName().map(|s| s.to_string());
            info.path = f.bundleURL().and_then(|u| u.path()).map(|s| s.to_string());
            pid = f.processIdentifier();
        }
        if let Some(b) = &info.bundle {
            if self.settings.borrow().ignore_apps.iter().any(|x| x == b) {
                log!("capture: ignored app {b}");
                return;
            }
        }
        let (window_title, source_path) = crate::ax::focused_window_info(pid);
        if let (Some(key), Some(path)) = (info.bundle.clone().or(info.name.clone()), info.path.clone()) {
            if self.seen_apps.borrow_mut().insert(key.clone()) {
                let _ = self.jobs.send(Job::SaveAppIcon { key, name: info.name.clone(), path });
            }
        }
        let cap = Capture { ts: timefmt::now_ms(), reps, app: info, window_title, source_path, starred: false };
        let _ = self.jobs.send(Job::Capture(cap));
    }

    fn on_event(&self, e: Event) {
        match e {
            Event::Ingested(id) | Event::Ocr(id) => {
                self.invalidate(id);
                let is_ocr = matches!(e, Event::Ocr(_));
                if is_ocr {
                    if self.selected_id() == Some(id) {
                        self.update_preview();
                    }
                    return;
                }
            }
            Event::Pruned(_) => self.invalidate_all(),
        }
        if self.ui().panel.isVisible() {
            self.run_search(false);
        } else {
            self.dirty.set(true);
        }
    }

    // ----------------------------------------------------------------- cache

    pub fn invalidate(&self, id: i64) {
        self.cache.borrow_mut().remove(&id);
    }

    pub fn invalidate_all(&self) {
        self.cache.borrow_mut().clear();
        self.cache_order.borrow_mut().clear();
    }

    pub fn row_data(&self, id: i64) -> Option<Rc<RowData>> {
        if let Some(d) = self.cache.borrow().get(&id) {
            return Some(d.clone());
        }
        let mut info = self.db.row(id).ok()??;
        let thumb = info
            .thumb
            .take()
            .and_then(|t| NSImage::initWithData(NSImage::alloc(), &NSData::from_vec(t)));
        let d = Rc::new(RowData { info, thumb });
        let mut cache = self.cache.borrow_mut();
        let mut order = self.cache_order.borrow_mut();
        cache.insert(id, d.clone());
        order.push_back(id);
        while order.len() > CACHE_ROWS {
            if let Some(old) = order.pop_front() {
                if old != id {
                    cache.remove(&old);
                }
            }
        }
        Some(d)
    }

    pub fn app_icon(&self, info: &RowInfo) -> Retained<NSImage> {
        let key = format!("app:{}", info.app_key);
        if let Some(i) = self.icons.borrow().get(&key) {
            return i.clone();
        }
        let img = info
            .app_path
            .as_deref()
            .filter(|p| std::path::Path::new(p).exists())
            .map(|p| NSWorkspace::sharedWorkspace().iconForFile(&NSString::from_str(p)))
            .or_else(|| {
                let png = self.db.app_icon(&info.app_key).ok()??;
                NSImage::initWithData(NSImage::alloc(), &NSData::from_vec(png))
            })
            .or_else(|| symbol("doc.on.clipboard"))
            .unwrap_or_else(NSImage::new);
        self.icons.borrow_mut().insert(key, img.clone());
        img
    }

    pub fn file_icon(&self, path: &str) -> Retained<NSImage> {
        let key = format!("file:{path}");
        if let Some(i) = self.icons.borrow().get(&key) {
            return i.clone();
        }
        let img = NSWorkspace::sharedWorkspace().iconForFile(&NSString::from_str(path));
        self.icons.borrow_mut().insert(key, img.clone());
        img
    }

    pub fn small_app_icon(&self, key: &str, path: Option<&str>) -> Option<Retained<NSImage>> {
        let ck = format!("small:{key}");
        if let Some(i) = self.icons.borrow().get(&ck) {
            return Some(i.clone());
        }
        let info = RowInfo {
            id: 0,
            kind: Kind::Other,
            preview: String::new(),
            starred: false,
            app_key: key.to_string(),
            app_name: None,
            app_path: path.map(String::from),
            copied_at: 0,
            img_w: 0,
            img_h: 0,
            img_scale: 1.0,
            thumb: None,
        };
        let big = self.app_icon(&info);
        let small: Retained<NSImage> = unsafe { msg_send![&*big, copy] };
        small.setSize(objc2_foundation::NSSize::new(16.0, 16.0));
        self.icons.borrow_mut().insert(ck, small.clone());
        Some(small)
    }

    // ----------------------------------------------------------------- list

    pub fn selected_id(&self) -> Option<i64> {
        let row = self.ui().table.selectedRow();
        if row < 0 {
            return None;
        }
        self.ids.borrow().get(row as usize).copied()
    }

    pub fn clicked_row(&self) -> isize {
        self.ui().table.clickedRow()
    }

    pub fn run_search(&self, reset_selection: bool) {
        let ui = self.ui();
        let q = ui.search.stringValue().to_string();
        let star = self.settings.borrow().star_filter;
        let app_f = self.app_filter.borrow().clone();
        let prev = if reset_selection { None } else { self.selected_id() };
        let t0 = Instant::now();
        let ids = self
            .db
            .search(&Filter { query: &q, starred: star, app: app_f.as_deref() }, SEARCH_LIMIT)
            .unwrap_or_else(|e| {
                log!("search {q:?}: {e}");
                Vec::new()
            });
        let row = prev.and_then(|id| ids.iter().position(|x| *x == id)).unwrap_or(0);
        let n = ids.len();
        *self.ids.borrow_mut() = ids;
        self.dirty.set(false);
        ui.table.reloadData();
        if n > 0 {
            self.select_row(row as isize);
        }
        self.update_preview();
        let label = if q.is_empty() && !star && app_f.is_none() {
            format!("{n}")
        } else {
            format!("{n} found")
        };
        ui.count_label.setStringValue(&NSString::from_str(&label));
        if t0.elapsed().as_millis() > 50 {
            log!("search {q:?}: {n} rows in {:?}", t0.elapsed());
        }
    }

    fn select_row(&self, row: isize) {
        let t = &self.ui().table;
        let n = self.ids.borrow().len() as isize;
        if n == 0 {
            return;
        }
        let row = row.clamp(0, n - 1);
        t.selectRowIndexes_byExtendingSelection(&NSIndexSet::indexSetWithIndex(row as usize), false);
        t.scrollRowToVisible(row);
    }

    fn move_selection(&self, delta: isize) {
        let cur = self.ui().table.selectedRow().max(0);
        self.select_row(cur + delta);
    }

    pub fn selection_changed(&self) {
        self.update_preview();
    }

    fn reload_row_of(&self, id: i64) {
        let pos = self.ids.borrow().iter().position(|x| *x == id);
        if let Some(row) = pos {
            self.ui().table.reloadDataForRowIndexes_columnIndexes(
                &NSIndexSet::indexSetWithIndex(row),
                &NSIndexSet::indexSetWithIndex(0),
            );
        }
    }

    // --------------------------------------------------------------- preview

    pub fn update_preview(&self) {
        let ui = self.ui();
        let Some(id) = self.selected_id() else {
            ui.image_view.setImage(None);
            ui.image_view.setHidden(true);
            ui.text_scroll.setHidden(false);
            set_text(&ui.text_view, "", false);
            ui.meta.setStringValue(&NSString::from_str(""));
            ui.date_label.setStringValue(&NSString::from_str(""));
            return;
        };
        let Ok(Some(d)) = self.db.detail(id) else { return };
        ui.date_label.setStringValue(&NSString::from_str(&timefmt::human(d.copied_at)));
        let mut shown_image = false;
        if d.kind == Kind::Image {
            let types = self.db.rep_types(id).unwrap_or_default();
            let uti = crate::ingest::IMAGE_TYPES.iter().find(|t| types.iter().any(|x| x == *t));
            if let Some(data) = uti.and_then(|u| self.db.rep(id, u).ok().flatten()) {
                if let Some(img) = NSImage::initWithData(NSImage::alloc(), &NSData::from_vec(data)) {
                    ui.image_view.setImage(Some(&img));
                    shown_image = true;
                }
            }
        }
        ui.image_view.setHidden(!shown_image);
        ui.text_scroll.setHidden(shown_image);
        if !shown_image {
            ui.image_view.setImage(None);
            self.show_text_preview(id, &d);
        }
        ui.meta.setStringValue(&NSString::from_str(&meta_text(&d)));
    }

    fn show_text_preview(&self, id: i64, d: &Detail) {
        let tv = &self.ui().text_view;
        if matches!(d.kind, Kind::Text | Kind::Url) {
            if let Ok(Some(rtf)) = self.db.rep(id, "public.rtf") {
                if rtf.len() < 2 * 1024 * 1024 {
                    let data = NSData::from_vec(rtf);
                    let s = unsafe {
                        NSAttributedString::initWithRTF_documentAttributes(NSAttributedString::alloc(), &data, None)
                    };
                    if let Some(s) = s {
                        if let Some(storage) = unsafe { tv.textStorage() } {
                            storage.setAttributedString(&s);
                        }
                        tv.setDrawsBackground(true);
                        tv.setBackgroundColor(&NSColor::whiteColor());
                        tv.scrollRangeToVisible(NSRange::new(0, 0));
                        return;
                    }
                }
            }
        }
        let mono = d.app_bundle.as_deref().is_some_and(is_code_app);
        let text = match d.kind {
            Kind::Other => format!("Data types:\n{}", d.types.replace(',', "\n")),
            _ => d.text.clone().unwrap_or_default(),
        };
        let shown: String = text.chars().take(1_000_000).collect();
        set_text(tv, &shown, mono);
    }

    // ----------------------------------------------------------------- show

    pub fn show(&self) {
        let ui = self.ui();
        let ws = NSWorkspace::sharedWorkspace();
        if let Some(front) = ws.frontmostApplication() {
            let bid = front.bundleIdentifier().map(|s| s.to_string());
            if bid.is_none() || bid != self.own_bundle {
                *self.prev_app.borrow_mut() = Some(front);
            }
        }
        *self.app_filter.borrow_mut() = None;
        self.rebuild_app_popup();
        ui.search.setStringValue(&NSString::from_str(""));
        self.run_search(true);
        self.position_panel();
        ui.panel.makeKeyAndOrderFront(None);
        ui.panel.makeFirstResponder(Some(&ui.table));
    }

    fn position_panel(&self) {
        let ui = self.ui();
        let panel = &ui.panel;
        let mut frame = panel.frame();
        let open_at = self.settings.borrow().open_at.clone();
        let remembered = NSUserDefaults::standardUserDefaults()
            .objectForKey(&NSString::from_str(&format!("NSWindow Frame {PANEL_AUTOSAVE}")))
            .is_some();
        let top_left: Option<NSPoint> = match open_at.as_str() {
            "mouse" => Some(NSEvent::mouseLocation()),
            "menubar" => self.status_anchor(),
            _ if !remembered => self.status_anchor(),
            _ => None,
        };
        if let Some(p) = top_left {
            frame.origin.x = p.x;
            frame.origin.y = p.y - frame.size.height;
        }
        // Keep it on a screen (monitors come and go).
        let screens = NSScreen::screens(self.mtm);
        let on_screen = screens.iter().find(|s| {
            let v = s.visibleFrame();
            frame.origin.x < v.origin.x + v.size.width - 40.0
                && frame.origin.x + frame.size.width > v.origin.x + 40.0
                && frame.origin.y < v.origin.y + v.size.height
                && frame.origin.y + frame.size.height > v.origin.y + 40.0
        });
        let screen = on_screen.or_else(|| NSScreen::mainScreen(self.mtm));
        if let Some(s) = screen {
            let v = s.visibleFrame();
            frame.size.width = frame.size.width.min(v.size.width);
            frame.size.height = frame.size.height.min(v.size.height);
            frame.origin.x = frame.origin.x.clamp(v.origin.x, v.origin.x + v.size.width - frame.size.width);
            frame.origin.y = frame.origin.y.clamp(v.origin.y, v.origin.y + v.size.height - frame.size.height);
        }
        panel.setFrame_display(frame, true);
    }

    fn status_anchor(&self) -> Option<NSPoint> {
        let b = self.ui().status_item.button(self.mtm)?;
        let w = b.window()?;
        let f = w.frame();
        Some(NSPoint::new(f.origin.x, f.origin.y - 4.0))
    }

    pub fn hide(&self) {
        let p = &self.ui().panel;
        if p.isVisible() {
            p.orderOut(None);
            self.last_hide.set(Some(Instant::now()));
        }
    }

    pub fn window_resigned_key(&self, n: &NSNotification) {
        let ui = self.ui();
        let is_panel = n
            .object()
            .is_some_and(|o| std::ptr::eq(&*o as *const AnyObject, &*ui.panel as *const ClipPanel as *const AnyObject));
        if is_panel && !self.settings.borrow().pinned && !self.pasting.get() {
            self.hide();
        }
    }

    pub fn escape(&self) {
        let s = &self.ui().search;
        if s.stringValue().length() > 0 {
            s.setStringValue(&NSString::from_str(""));
            self.run_search(true);
            self.ui().panel.makeFirstResponder(Some(&self.ui().table));
        } else {
            self.hide();
        }
    }

    // ---------------------------------------------------------------- paste

    pub fn paste_selected(&self, plain: bool) {
        let Some(id) = self.selected_id() else { return };
        self.paste_id(id, plain);
    }

    fn write_clip(&self, id: i64, plain: bool) -> bool {
        let pb = NSPasteboard::generalPasteboard();
        let cc = if plain {
            match self.db.detail(id).ok().flatten().and_then(|d| d.text) {
                Some(t) => crate::paste::plain_text(&pb, &t),
                None => return false,
            }
        } else {
            match self.db.reps(id) {
                Ok(reps) if !reps.is_empty() => crate::paste::write(&pb, &reps),
                _ => return false,
            }
        };
        self.last_change.set(cc);
        true
    }

    fn paste_id(&self, id: i64, plain: bool) {
        if !self.write_clip(id, plain) {
            beep();
            return;
        }
        let move_top = self.settings.borrow().move_to_top;
        let _ = self.db.touch_pasted(id, timefmt::now_ms(), move_top);
        self.invalidate(id);
        self.dirty.set(true);
        self.pasting.set(true);
        self.ui().panel.orderOut(None);
        let nsapp = NSApplication::sharedApplication(self.mtm);
        if nsapp.isActive() {
            if let Some(prev) = self.prev_app.borrow().as_ref() {
                prev.activateWithOptions(NSApplicationActivationOptions::empty());
            }
        }
        unsafe {
            let _: () = msg_send![&*self.handler, performSelector: sel!(postPaste:), withObject: Option::<&AnyObject>::None, afterDelay: 0.12f64];
        }
    }

    pub fn post_paste(&self) {
        self.pasting.set(false);
        if crate::ax::trusted() {
            crate::paste::post_cmd_v();
        } else {
            log!("paste: no accessibility permission; clip left on the clipboard");
            crate::ax::prompt();
        }
        if self.settings.borrow().pinned {
            self.run_search(true);
            self.ui().panel.orderFrontRegardless();
        }
    }

    pub fn copy_selected(&self) {
        if let Some(id) = self.selected_id() {
            if self.write_clip(id, false) {
                self.hide();
            }
        }
    }

    // ------------------------------------------------------------- stars etc

    pub fn toggle_star(&self, id: i64) {
        let starred = self.db.row(id).ok().flatten().is_some_and(|r| r.starred);
        let _ = self.db.set_star(id, !starred);
        self.invalidate(id);
        if self.settings.borrow().star_filter {
            self.run_search(false);
        } else {
            self.reload_row_of(id);
            if self.selected_id() == Some(id) {
                self.update_preview();
            }
        }
    }

    pub fn toggle_star_filter(&self) {
        {
            let mut s = self.settings.borrow_mut();
            s.star_filter = !s.star_filter;
            s.save(&self.db);
        }
        self.apply_buttons();
        self.run_search(true);
    }

    pub fn toggle_pin(&self) {
        {
            let mut s = self.settings.borrow_mut();
            s.pinned = !s.pinned;
            s.save(&self.db);
        }
        self.apply_buttons();
    }

    pub fn apply_buttons(&self) {
        let ui = self.ui();
        let (star, pin) = {
            let s = self.settings.borrow();
            (s.star_filter, s.pinned)
        };
        let star_img = if star {
            draw::tinted_symbol("star.fill", &NSColor::systemYellowColor(), 14.0)
        } else {
            draw::tinted_symbol("star", &NSColor::secondaryLabelColor(), 14.0)
        };
        ui.star_btn.setImage(star_img.as_deref());
        ui.star_btn.setToolTip(Some(&NSString::from_str(if star {
            "Showing starred only (S)"
        } else {
            "Show starred only (S)"
        })));
        let pin_img = if pin {
            draw::tinted_symbol("pin.fill", &NSColor::controlAccentColor(), 14.0)
        } else {
            draw::tinted_symbol("pin", &NSColor::secondaryLabelColor(), 14.0)
        };
        ui.pin_btn.setImage(pin_img.as_deref());
        ui.pin_btn.setToolTip(Some(&NSString::from_str(if pin {
            "Pinned: stays open (⌘P)"
        } else {
            "Pin window open (⌘P)"
        })));
        ui.table.setRowHeight(self.settings.borrow().row_height);
    }

    pub fn delete_selected(&self) {
        let Some(id) = self.selected_id() else { return };
        let row = self.ui().table.selectedRow();
        let starred = self.db.row(id).ok().flatten().is_some_and(|r| r.starred);
        if starred && !self.confirm("Delete this starred clip?", "It will be removed from the history for good.", "Delete") {
            return;
        }
        if let Err(e) = self.db.delete(id) {
            log!("delete {id}: {e}");
            return;
        }
        self.invalidate(id);
        self.ids.borrow_mut().retain(|x| *x != id);
        self.ui().table.reloadData();
        self.select_row(row);
        self.update_preview();
    }

    pub fn app_filter_changed(&self) {
        let item = self.ui().app_popup.selectedItem();
        let key = item.and_then(|i| i.representedObject()).and_then(|o| o.downcast::<NSString>().ok());
        *self.app_filter.borrow_mut() = key.map(|k| k.to_string());
        self.run_search(true);
    }

    fn rebuild_app_popup(&self) {
        let pop = &self.ui().app_popup;
        pop.removeAllItems();
        pop.addItemWithTitle(&NSString::from_str("All Applications"));
        if let Some(menu) = pop.menu() {
            menu.addItem(&objc2_app_kit::NSMenuItem::separatorItem(self.mtm));
        }
        for a in self.db.apps().unwrap_or_default() {
            let title = NSString::from_str(&format!("{}  ({})", a.name.trim_start_matches('\u{200e}'), a.count));
            pop.addItemWithTitle(&title);
            if let Some(item) = pop.lastItem() {
                unsafe { item.setRepresentedObject(Some(&NSString::from_str(&a.key))) };
                item.setImage(self.small_app_icon(&a.key, a.path.as_deref()).as_deref());
            }
        }
        pop.selectItemAtIndex(0);
    }

    // ------------------------------------------------------------- keyboard

    /// Cmd-shortcuts anywhere in the panel.
    pub fn key_equivalent(&self, ev: &NSEvent) -> bool {
        let flags = ev.modifierFlags();
        if !flags.contains(NSEventModifierFlags::Command) {
            return false;
        }
        let shift = flags.contains(NSEventModifierFlags::Shift);
        let chars = ev.charactersIgnoringModifiers().map(|s| s.to_string()).unwrap_or_default().to_lowercase();
        let ui = self.ui();
        let table_focused = ui
            .panel
            .firstResponder()
            .is_some_and(|r| std::ptr::eq(&*r as *const _ as *const AnyObject, &*ui.table as *const ClipTable as *const AnyObject));
        let nsapp = NSApplication::sharedApplication(self.mtm);
        let send = |s: Sel| unsafe { nsapp.sendAction_to_from(s, None, Some(&ui.panel)) };
        match chars.as_str() {
            "f" => {
                ui.panel.makeFirstResponder(Some(&ui.search));
                true
            }
            "," => {
                prefs::show();
                true
            }
            "p" => {
                self.toggle_pin();
                true
            }
            "w" => {
                self.hide();
                true
            }
            "q" => {
                self.quit();
                true
            }
            "s" => {
                if let Some(id) = self.selected_id() {
                    self.toggle_star(id);
                }
                true
            }
            "\r" => {
                self.copy_selected();
                true
            }
            "\u{7f}" | "\u{8}" => {
                self.delete_selected();
                true
            }
            "c" if table_focused => {
                self.copy_selected();
                true
            }
            "c" => send(sel!(copy:)),
            "x" => send(sel!(cut:)),
            "v" => send(sel!(paste:)),
            "a" => send(sel!(selectAll:)),
            "z" if shift => send(sel!(redo:)),
            "z" => send(sel!(undo:)),
            d if d.len() == 1 && ("1"..="9").contains(&d) => {
                let n: usize = d.parse().unwrap_or(1);
                let id = self.ids.borrow().get(n - 1).copied();
                if let Some(id) = id {
                    self.paste_id(id, false);
                }
                true
            }
            _ => false,
        }
    }

    /// Plain keys while the list has focus.
    pub fn table_key(&self, ev: &NSEvent) -> bool {
        let flags = ev.modifierFlags();
        if flags.intersects(NSEventModifierFlags::Command | NSEventModifierFlags::Control) {
            return false;
        }
        let code = ev.keyCode();
        let ui = self.ui();
        match code {
            36 | 76 => {
                self.paste_selected(flags.contains(NSEventModifierFlags::Shift));
                return true;
            }
            53 => {
                self.escape();
                return true;
            }
            51 | 117 => {
                self.delete_selected();
                return true;
            }
            48 => {
                ui.panel.makeFirstResponder(Some(&ui.search));
                return true;
            }
            _ => {}
        }
        let chars = ev.characters().map(|s| s.to_string()).unwrap_or_default();
        match chars.as_str() {
            "s" | "S" => {
                self.toggle_star_filter();
                true
            }
            " " => {
                if let Some(id) = self.selected_id() {
                    self.toggle_star(id);
                }
                true
            }
            "/" => {
                ui.panel.makeFirstResponder(Some(&ui.search));
                true
            }
            c if c.chars().count() == 1 && c.chars().all(|ch| !ch.is_control() && !('\u{f700}'..='\u{f8ff}').contains(&ch)) => {
                // Start typing a search right away.
                ui.panel.makeFirstResponder(Some(&ui.search));
                if let Some(ed) = ui.search.currentEditor() {
                    let len = ui.search.stringValue().length();
                    ed.setSelectedRange(NSRange::new(len, 0));
                    unsafe {
                        let _: () = msg_send![&*ed, insertText: &*NSString::from_str(c)];
                    }
                }
                true
            }
            _ => false,
        }
    }

    pub fn text_changed(&self, n: &NSNotification) {
        let ui = self.ui();
        let from_search = n
            .object()
            .is_some_and(|o| std::ptr::eq(&*o as *const AnyObject, &*ui.search as *const NSSearchField as *const AnyObject));
        if from_search {
            self.run_search(true);
        }
    }

    pub fn search_command(&self, control: &NSControl, sel: Sel) -> bool {
        let ui = self.ui();
        if !std::ptr::eq(control as *const NSControl as *const AnyObject, &*ui.search as *const NSSearchField as *const AnyObject) {
            return false;
        }
        let shift = NSApplication::sharedApplication(self.mtm)
            .currentEvent()
            .is_some_and(|e| e.modifierFlags().contains(NSEventModifierFlags::Shift));
        if sel == sel!(moveUp:) {
            self.move_selection(-1);
        } else if sel == sel!(moveDown:) {
            self.move_selection(1);
        } else if sel == sel!(pageUp:) || sel == sel!(scrollPageUp:) {
            self.move_selection(-10);
        } else if sel == sel!(pageDown:) || sel == sel!(scrollPageDown:) {
            self.move_selection(10);
        } else if sel == sel!(insertNewline:) {
            self.paste_selected(shift);
        } else if sel == sel!(cancelOperation:) {
            self.escape();
        } else if sel == sel!(insertTab:) {
            ui.panel.makeFirstResponder(Some(&ui.table));
        } else {
            return false;
        }
        true
    }

    // ------------------------------------------------------------ gear menu

    pub fn confirm(&self, title: &str, text: &str, ok: &str) -> bool {
        let nsapp = NSApplication::sharedApplication(self.mtm);
        nsapp.activate();
        let alert = NSAlert::new(self.mtm);
        alert.setMessageText(&NSString::from_str(title));
        alert.setInformativeText(&NSString::from_str(text));
        alert.addButtonWithTitle(&NSString::from_str(ok));
        alert.addButtonWithTitle(&NSString::from_str("Cancel"));
        alert.runModal() == NSAlertFirstButtonReturn
    }

    pub fn clear_history(&self, tag: isize) {
        let (before, starred, what) = match tag {
            d if d > 0 => (
                Some(timefmt::now_ms() - d as i64 * 86_400_000),
                false,
                format!("unstarred clips older than {d} day{}", if d == 1 { "" } else { "s" }),
            ),
            -1 => (None, false, "all unstarred clips".to_string()),
            -2 => (None, true, "EVERYTHING, starred clips included".to_string()),
            _ => return,
        };
        if !self.confirm(&format!("Delete {what}?"), "This cannot be undone.", "Delete") {
            return;
        }
        match self.db.clear(before, starred) {
            Ok(n) => log!("clear: removed {n} ({what})"),
            Err(e) => log!("clear: {e}"),
        }
        let _ = self.jobs.send(Job::Vacuum);
        self.invalidate_all();
        self.run_search(true);
    }

    pub fn show_about(&self) {
        let nsapp = NSApplication::sharedApplication(self.mtm);
        nsapp.activate();
        let count = self.db.count().unwrap_or(0);
        let credits = format!(
            "A forever clipboard history.\n{count} clips · {} on disk\n{}",
            timefmt::bytes(self.db.file_size()),
            Db::default_path().display()
        );
        let key = NSString::from_str("Credits");
        let val = NSAttributedString::from_nsstring(&NSString::from_str(&credits));
        let dict = objc2_foundation::NSDictionary::<NSString, AnyObject>::from_slices(&[&*key], &[&**val as &AnyObject]);
        unsafe {
            let _: () = msg_send![&*nsapp, orderFrontStandardAboutPanelWithOptions: &*dict];
        }
    }

    pub fn quit(&self) {
        log!("quit requested");
        NSApplication::sharedApplication(self.mtm).terminate(None);
    }
}

// ------------------------------------------------------------------ helpers

#[link(name = "AppKit", kind = "framework")]
unsafe extern "C" {
    #[link_name = "NSBeep"]
    fn ns_beep();
}

fn beep() {
    unsafe { ns_beep() }
}

fn post_distributed(name: &str) {
    unsafe {
        NSDistributedNotificationCenter::defaultCenter().postNotificationName_object_userInfo_deliverImmediately(
            &NSString::from_str(name),
            None,
            None,
            true,
        );
    }
}

pub fn symbol(name: &str) -> Option<Retained<NSImage>> {
    NSImage::imageWithSystemSymbolName_accessibilityDescription(&NSString::from_str(name), None)
}

fn is_code_app(bundle: &str) -> bool {
    let b = bundle.to_lowercase();
    ["iterm", "terminal", "vscode", "zed", "xcode", "ghostty", "warp", "alacritty", "kitty", "sublime", "jetbrains", "cursor", "nova", "bbedit", "neovim"]
        .iter()
        .any(|k| b.contains(k))
}

fn set_text(tv: &NSTextView, text: &str, mono: bool) {
    tv.setDrawsBackground(false);
    tv.setString(&NSString::from_str(text));
    let font = if mono {
        NSFont::monospacedSystemFontOfSize_weight(12.0, 0.0)
    } else {
        NSFont::systemFontOfSize(13.0)
    };
    tv.setFont(Some(&font));
    tv.setTextColor(Some(&NSColor::labelColor()));
    tv.scrollRangeToVisible(NSRange::new(0, 0));
}

fn meta_text(d: &Detail) -> String {
    let mut lines = Vec::new();
    let app = d.app_name.clone().unwrap_or_else(|| "Unknown app".into());
    match &d.window_title {
        Some(t) => lines.push(format!("{} — {}", app.trim_start_matches('\u{200e}'), t)),
        None => lines.push(app.trim_start_matches('\u{200e}').to_string()),
    }
    if let Some(p) = &d.source_path {
        lines.push(format!("File: {p}"));
    }
    if let Some(u) = &d.source_url {
        lines.push(format!("From: {u}"));
    }
    let mut when = format!("Copied {}", timefmt::full(d.copied_at));
    if d.created_at + 1000 < d.copied_at {
        when += &format!(" · first {}", timefmt::full(d.created_at));
    }
    if d.copy_count > 1 {
        when += &format!(" · copied {}×", d.copy_count);
    }
    if d.paste_count > 0 {
        when += &format!(" · pasted {}×", d.paste_count);
    }
    lines.push(when);
    let mut what = match d.kind {
        Kind::Image => format!("Image {}×{} px", d.img_w, d.img_h),
        Kind::Text | Kind::Url => {
            let t = d.text.as_deref().unwrap_or("");
            format!("{} chars · {} lines", t.chars().count(), t.lines().count().max(1))
        }
        Kind::File => format!("{} file(s)", d.text.as_deref().map(|t| t.lines().count()).unwrap_or(0)),
        Kind::Other => "Other data".into(),
    };
    what += &format!(
        " · {} → {} stored · {} types",
        timefmt::bytes(d.raw_size),
        timefmt::bytes(d.stored_size),
        d.types.split(',').filter(|s| !s.is_empty()).count()
    );
    if d.starred {
        what += " · ★";
    }
    lines.push(what);
    if let Some(o) = &d.ocr_text {
        let one: String = o.split_whitespace().collect::<Vec<_>>().join(" ");
        let short: String = one.chars().take(240).collect();
        lines.push(format!("Text in image: {short}{}", if one.chars().count() > 240 { "…" } else { "" }));
    }
    lines.join("\n")
}

/// Table delegate helper: a (reused) row view bound to the clip at `row`.
pub(crate) fn row_view_for(tv: &NSTableView, row: isize) -> Retained<NSView> {
    let mtm = MainThreadMarker::new().unwrap();
    let ident = NSString::from_str("ClipRow");
    let reused = unsafe { tv.makeViewWithIdentifier_owner(&ident, None) }.and_then(|v| v.downcast::<RowView>().ok());
    let v = reused.unwrap_or_else(|| {
        let v = RowView::new(mtm, NSRect::ZERO);
        v.setIdentifier(Some(&ident));
        v
    });
    let id = app().ids.borrow().get(row as usize).copied().unwrap_or(0);
    v.ivars().clip.set(id);
    v.setNeedsDisplay(true);
    Retained::into_super(v)
}

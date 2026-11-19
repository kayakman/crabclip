//! Objective-C classes: the delegate/target object, the panel, the clip table,
//! its custom-drawn row view and the always-emphasized row background.

use super::app;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol, Sel};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{
    NSApplication, NSApplicationDelegate, NSBackgroundStyle, NSControl, NSControlTextEditingDelegate,
    NSEvent, NSMenuDelegate, NSPanel, NSSearchFieldDelegate, NSTableRowView, NSTableView,
    NSTableViewDataSource, NSTableViewDelegate, NSTextFieldDelegate, NSTextView, NSView,
    NSWindowDelegate, NSTableColumn,
};
use objc2_foundation::{NSInteger, NSNotification, NSPoint, NSRect};
use std::cell::Cell;

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "CliprustHandler"]
    pub struct Handler;

    unsafe impl NSObjectProtocol for Handler {}

    unsafe impl NSApplicationDelegate for Handler {
        #[unsafe(method(applicationDidFinishLaunching:))]
        fn did_finish_launching(&self, _n: &NSNotification) {
            super::start(self);
        }

        #[unsafe(method(applicationShouldHandleReopen:hasVisibleWindows:))]
        fn should_reopen(&self, _app: &NSApplication, _visible: bool) -> bool {
            app().show();
            false
        }
    }

    unsafe impl NSTableViewDataSource for Handler {
        #[unsafe(method(numberOfRowsInTableView:))]
        fn number_of_rows(&self, _tv: &NSTableView) -> NSInteger {
            app().ids.borrow().len() as NSInteger
        }
    }

    unsafe impl NSTableViewDelegate for Handler {
        #[unsafe(method_id(tableView:viewForTableColumn:row:))]
        fn view_for_row(
            &self,
            tv: &NSTableView,
            _col: Option<&NSTableColumn>,
            row: NSInteger,
        ) -> Option<Retained<NSView>> {
            Some(super::row_view_for(tv, row))
        }

        #[unsafe(method_id(tableView:rowViewForRow:))]
        fn row_bg_for_row(&self, _tv: &NSTableView, _row: NSInteger) -> Option<Retained<NSTableRowView>> {
            let mtm = MainThreadMarker::new().unwrap();
            let v: Retained<RowBg> = unsafe { msg_send![RowBg::alloc(mtm), init] };
            Some(Retained::into_super(v))
        }

        #[unsafe(method(tableViewSelectionDidChange:))]
        fn selection_changed(&self, _n: &NSNotification) {
            app().selection_changed();
        }
    }

    unsafe impl NSControlTextEditingDelegate for Handler {
        #[unsafe(method(controlTextDidChange:))]
        fn control_text_did_change(&self, n: &NSNotification) {
            app().text_changed(n);
        }

        #[unsafe(method(control:textView:doCommandBySelector:))]
        fn do_command(&self, control: &NSControl, _tv: &NSTextView, sel: Sel) -> bool {
            app().search_command(control, sel)
        }
    }

    unsafe impl NSTextFieldDelegate for Handler {}
    unsafe impl NSSearchFieldDelegate for Handler {}

    unsafe impl NSWindowDelegate for Handler {
        #[unsafe(method(windowDidResignKey:))]
        fn window_did_resign_key(&self, n: &NSNotification) {
            app().window_resigned_key(n);
        }

        #[unsafe(method(windowWillClose:))]
        fn window_will_close(&self, n: &NSNotification) {
            super::prefs::closing(n);
        }

        #[unsafe(method(windowDidResize:))]
        fn window_did_resize(&self, _n: &NSNotification) {
            app().apply_scale();
        }
    }

    unsafe impl NSMenuDelegate for Handler {}

    impl Handler {
        #[unsafe(method(tick:))]
        fn tick(&self, _t: Option<&AnyObject>) {
            app().tick();
        }

        #[unsafe(method(statusClicked:))]
        fn status_clicked(&self, _s: Option<&AnyObject>) {
            app().status_clicked();
        }

        #[unsafe(method(closePanel:))]
        fn close_panel(&self, _s: Option<&AnyObject>) {
            app().hide();
        }

        #[unsafe(method(toggleStarFilter:))]
        fn toggle_star_filter(&self, _s: Option<&AnyObject>) {
            app().toggle_star_filter();
        }

        #[unsafe(method(togglePin:))]
        fn toggle_pin(&self, _s: Option<&AnyObject>) {
            app().toggle_pin();
        }

        #[unsafe(method(togglePreview:))]
        fn toggle_preview(&self, _s: Option<&AnyObject>) {
            app().toggle_preview();
        }

        #[unsafe(method(appFilterChanged:))]
        fn app_filter_changed(&self, _s: Option<&AnyObject>) {
            app().app_filter_changed();
        }

        #[unsafe(method(searchChanged:))]
        fn search_changed(&self, _s: Option<&AnyObject>) {
            app().run_search(true);
        }

        #[unsafe(method(rowDoubleClicked:))]
        fn row_double_clicked(&self, _s: Option<&AnyObject>) {
            if app().clicked_row() >= 0 {
                app().enter(false, false);
            }
        }

        #[unsafe(method(showPreferences:))]
        fn show_preferences(&self, _s: Option<&AnyObject>) {
            super::prefs::show();
        }

        #[unsafe(method(showAbout:))]
        fn show_about(&self, _s: Option<&AnyObject>) {
            app().show_about();
        }

        #[unsafe(method(clearHistory:))]
        fn clear_history(&self, sender: Option<&AnyObject>) {
            let tag: NSInteger = sender.map(|s| unsafe { msg_send![s, tag] }).unwrap_or(0);
            app().clear_history(tag);
        }

        #[unsafe(method(quit:))]
        fn quit(&self, _s: Option<&AnyObject>) {
            app().quit();
        }

        #[unsafe(method(prefChanged:))]
        fn pref_changed(&self, sender: Option<&AnyObject>) {
            super::prefs::changed(sender);
        }

        #[unsafe(method(prefAction:))]
        fn pref_action(&self, sender: Option<&AnyObject>) {
            super::prefs::action(sender);
        }

        #[unsafe(method(postPaste:))]
        fn post_paste(&self, _s: Option<&AnyObject>) {
            app().post_paste();
        }

        #[unsafe(method(quitFromNotification:))]
        fn quit_from_notification(&self, _n: Option<&AnyObject>) {
            crate::log!("another instance (login agent) takes over");
            app().quit();
        }

        #[unsafe(method(showFromNotification:))]
        fn show_from_notification(&self, _n: Option<&AnyObject>) {
            app().show();
        }
    }
);

impl Handler {
    pub fn new(mtm: MainThreadMarker) -> Retained<Self> {
        unsafe { msg_send![Self::alloc(mtm), init] }
    }
}

define_class!(
    #[unsafe(super(NSPanel))]
    #[thread_kind = MainThreadOnly]
    #[name = "CliprustPanel"]
    pub struct ClipPanel;

    impl ClipPanel {
        #[unsafe(method(canBecomeKeyWindow))]
        fn can_become_key(&self) -> bool {
            true
        }

        #[unsafe(method(canBecomeMainWindow))]
        fn can_become_main(&self) -> bool {
            false
        }

        #[unsafe(method(performKeyEquivalent:))]
        fn perform_key_equivalent(&self, ev: &NSEvent) -> bool {
            app().key_equivalent(ev) || unsafe { msg_send![super(self), performKeyEquivalent: ev] }
        }

        #[unsafe(method(cancelOperation:))]
        fn cancel_operation(&self, _s: Option<&AnyObject>) {
            app().escape();
        }
    }
);

define_class!(
    #[unsafe(super(NSTableView))]
    #[thread_kind = MainThreadOnly]
    #[name = "CliprustTable"]
    pub struct ClipTable;

    impl ClipTable {
        #[unsafe(method(keyDown:))]
        fn key_down(&self, ev: &NSEvent) {
            if !app().table_key(ev) {
                unsafe {
                    let _: () = msg_send![super(self), keyDown: ev];
                }
            }
        }

        #[unsafe(method(copy:))]
        fn copy(&self, _s: Option<&AnyObject>) {
            app().copy_selected();
        }

        #[unsafe(method(validateUserInterfaceItem:))]
        fn validate(&self, _item: &AnyObject) -> bool {
            true
        }
    }
);

pub struct RowIvars {
    pub clip: Cell<i64>,
    pub selected: Cell<bool>,
}

define_class!(
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[name = "CliprustRowView"]
    #[ivars = RowIvars]
    pub struct RowView;

    impl RowView {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, _dirty: NSRect) {
            super::draw::row(self);
        }

        #[unsafe(method(setBackgroundStyle:))]
        fn set_background_style(&self, style: NSBackgroundStyle) {
            let sel = style == NSBackgroundStyle::Emphasized;
            if sel != self.ivars().selected.get() {
                self.ivars().selected.set(sel);
                self.setNeedsDisplay(true);
            }
        }

        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, ev: &NSEvent) {
            let p: NSPoint = self.convertPoint_fromView(ev.locationInWindow(), None);
            let r = super::draw::star_rect(self.bounds());
            let inside = p.x >= r.origin.x && p.x <= r.origin.x + r.size.width && p.y >= r.origin.y && p.y <= r.origin.y + r.size.height;
            if inside {
                app().toggle_star(self.ivars().clip.get());
                return;
            }
            unsafe {
                let _: () = msg_send![super(self), mouseDown: ev];
            }
        }
    }
);

impl RowView {
    pub fn new(mtm: MainThreadMarker, frame: NSRect) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(RowIvars { clip: Cell::new(0), selected: Cell::new(false) });
        unsafe { msg_send![super(this), initWithFrame: frame] }
    }
}

define_class!(
    #[unsafe(super(NSTableRowView))]
    #[thread_kind = MainThreadOnly]
    #[name = "CliprustRowBg"]
    pub struct RowBg;

    impl RowBg {
        // Keep the blue selection even while the search field has focus.
        #[unsafe(method(isEmphasized))]
        fn is_emphasized(&self) -> bool {
            true
        }
    }
);

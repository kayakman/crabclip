//! Builds the panel (top bar, list | preview split, bottom bar), the gear and
//! status menus, the status item and the (hidden) main menu.

use super::classes::{ClipPanel, ClipTable};
use super::{App, PANEL_AUTOSAVE, Ui, draw};
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject, Sel};
use objc2::{AnyThread, MainThreadMarker, MainThreadOnly, msg_send, sel};
use objc2_app_kit::{
    NSApplication, NSAutoresizingMaskOptions, NSBackingStoreType, NSBezelStyle, NSBorderType,
    NSButton, NSColor, NSEventMask, NSFocusRingType, NSFont, NSImage, NSImageAlignment,
    NSImageScaling, NSImageView, NSMenu, NSMenuItem, NSPopUpButton, NSScrollView, NSSearchField,
    NSSplitView, NSSplitViewDividerStyle, NSStatusBar, NSTableColumn, NSTableColumnResizingOptions,
    NSTableViewColumnAutoresizingStyle, NSTableViewGridLineStyle, NSTableViewStyle, NSTextAlignment,
    NSTextField, NSTextView, NSView, NSVisualEffectBlendingMode, NSVisualEffectMaterial,
    NSVisualEffectState, NSVisualEffectView, NSWindowButton, NSWindowCollectionBehavior,
    NSWindowStyleMask, NSWindowTitleVisibility,
};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};

pub const TOP: f64 = 38.0;
pub const BOTTOM: f64 = 28.0;

fn rect(x: f64, y: f64, w: f64, h: f64) -> NSRect {
    NSRect::new(NSPoint::new(x, y), NSSize::new(w, h))
}

fn ns(s: &str) -> Retained<NSString> {
    NSString::from_str(s)
}

fn icon_button(mtm: MainThreadMarker, frame: NSRect, symbol: &str, tip: &str, target: &AnyObject, action: Sel) -> Retained<NSButton> {
    let b = NSButton::initWithFrame(NSButton::alloc(mtm), frame);
    b.setBezelStyle(NSBezelStyle::Toolbar);
    b.setBordered(false);
    b.setImage(draw::tinted_symbol(symbol, &NSColor::secondaryLabelColor(), 14.0).as_deref());
    b.setToolTip(Some(&ns(tip)));
    unsafe {
        b.setTarget(Some(target));
        b.setAction(Some(action));
    }
    b.setFocusRingType(NSFocusRingType::None);
    b
}

fn label(mtm: MainThreadMarker, frame: NSRect, size: f64) -> Retained<NSTextField> {
    let l = NSTextField::labelWithString(&ns(""), mtm);
    l.setFrame(frame);
    l.setFont(Some(&NSFont::systemFontOfSize(size)));
    l.setTextColor(Some(&NSColor::secondaryLabelColor()));
    l
}

pub fn menu_item(mtm: MainThreadMarker, title: &str, action: Option<Sel>, key: &str, target: Option<&AnyObject>, tag: isize) -> Retained<NSMenuItem> {
    let item = unsafe { NSMenuItem::initWithTitle_action_keyEquivalent(NSMenuItem::alloc(mtm), &ns(title), action, &ns(key)) };
    unsafe { item.setTarget(target) };
    item.setTag(tag);
    item
}

/// Preferences / About / Clear ▸ / Quit, shared by the gear button and a right
/// click on the menu bar icon.
pub fn gear_menu(a: &App, with_face: bool) -> Retained<NSMenu> {
    let mtm = a.mtm;
    let h: &AnyObject = &a.handler;
    let menu = NSMenu::new(mtm);
    menu.setAutoenablesItems(false);
    if with_face {
        // A pull-down popup shows its first item as the button face.
        let face = menu_item(mtm, "", None, "", None, 0);
        face.setImage(draw::tinted_symbol("gearshape", &NSColor::secondaryLabelColor(), 13.0).as_deref());
        menu.addItem(&face);
    }
    menu.addItem(&menu_item(mtm, "Preferences…", Some(sel!(showPreferences:)), ",", Some(h), 0));
    menu.addItem(&NSMenuItem::separatorItem(mtm));
    menu.addItem(&menu_item(mtm, "About cliprust", Some(sel!(showAbout:)), "", Some(h), 0));
    menu.addItem(&NSMenuItem::separatorItem(mtm));
    let clear = menu_item(mtm, "Clear", None, "", None, 0);
    let sub = NSMenu::new(mtm);
    for (title, tag) in [
        ("Older than 1 day…", 1),
        ("Older than 1 week…", 7),
        ("Older than 1 month…", 30),
        ("Older than 1 year…", 365),
    ] {
        sub.addItem(&menu_item(mtm, title, Some(sel!(clearHistory:)), "", Some(h), tag));
    }
    sub.addItem(&NSMenuItem::separatorItem(mtm));
    sub.addItem(&menu_item(mtm, "All unstarred…", Some(sel!(clearHistory:)), "", Some(h), -1));
    sub.addItem(&menu_item(mtm, "Everything, starred too…", Some(sel!(clearHistory:)), "", Some(h), -2));
    clear.setSubmenu(Some(&sub));
    menu.addItem(&clear);
    menu.addItem(&NSMenuItem::separatorItem(mtm));
    menu.addItem(&menu_item(mtm, "Quit cliprust", Some(sel!(quit:)), "q", Some(h), 0));
    menu
}

/// Hidden main menu so Cmd-X/C/V/A/Z work in text fields of an accessory app.
pub fn main_menu(a: &App) {
    let mtm = a.mtm;
    let bar = NSMenu::new(mtm);
    let app_item = NSMenuItem::new(mtm);
    let app_menu = NSMenu::new(mtm);
    app_menu.addItem(&menu_item(mtm, "Preferences…", Some(sel!(showPreferences:)), ",", Some(&a.handler), 0));
    app_menu.addItem(&menu_item(mtm, "Quit cliprust", Some(sel!(quit:)), "q", Some(&a.handler), 0));
    app_item.setSubmenu(Some(&app_menu));
    bar.addItem(&app_item);
    let edit_item = NSMenuItem::new(mtm);
    let edit = NSMenu::initWithTitle(NSMenu::alloc(mtm), &ns("Edit"));
    for (t, s, k) in [
        ("Undo", sel!(undo:), "z"),
        ("Redo", sel!(redo:), "Z"),
        ("Cut", sel!(cut:), "x"),
        ("Copy", sel!(copy:), "c"),
        ("Paste", sel!(paste:), "v"),
        ("Select All", sel!(selectAll:), "a"),
    ] {
        edit.addItem(&menu_item(mtm, t, Some(s), k, None, 0));
    }
    edit_item.setSubmenu(Some(&edit));
    bar.addItem(&edit_item);
    NSApplication::sharedApplication(mtm).setMainMenu(Some(&bar));
}

fn status_image() -> Option<Retained<NSImage>> {
    let img = NSImage::imageNamed(&ns("StatusIcon")).or_else(|| {
        // Running unbundled (cargo run): use the asset next to the sources.
        let p = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/StatusIcon@2x.png");
        NSImage::initWithContentsOfFile(NSImage::alloc(), &ns(p))
    })?;
    img.setSize(NSSize::new(18.0, 18.0));
    img.setTemplate(true);
    Some(img)
}

pub fn build(a: &App) -> Ui {
    let mtm = a.mtm;
    let h: &AnyObject = &a.handler;

    let style = NSWindowStyleMask::Titled
        | NSWindowStyleMask::Resizable
        | NSWindowStyleMask::FullSizeContentView
        | NSWindowStyleMask::NonactivatingPanel
        | NSWindowStyleMask::UtilityWindow;
    let panel: Retained<ClipPanel> = unsafe {
        msg_send![ClipPanel::alloc(mtm), initWithContentRect: rect(200.0, 300.0, 880.0, 560.0), styleMask: style, backing: NSBackingStoreType::Buffered, defer: false]
    };
    panel.setTitle(&ns("cliprust"));
    panel.setTitleVisibility(NSWindowTitleVisibility::Hidden);
    panel.setTitlebarAppearsTransparent(true);
    for b in [NSWindowButton::CloseButton, NSWindowButton::MiniaturizeButton, NSWindowButton::ZoomButton] {
        if let Some(btn) = panel.standardWindowButton(b) {
            btn.setHidden(true);
        }
    }
    panel.setFloatingPanel(true);
    panel.setLevel(3); // NSFloatingWindowLevel
    panel.setCollectionBehavior(
        NSWindowCollectionBehavior::CanJoinAllSpaces | NSWindowCollectionBehavior::FullScreenAuxiliary,
    );
    panel.setHidesOnDeactivate(false);
    unsafe { panel.setReleasedWhenClosed(false) };
    panel.setMovableByWindowBackground(true);
    panel.setMinSize(NSSize::new(300.0, 280.0)); // apply_scale sets the real one
    panel.setDelegate(Some(ProtocolObject::from_ref(&*a.handler)));
    panel.setFrameUsingName(&ns(PANEL_AUTOSAVE));
    panel.setFrameAutosaveName(&ns(PANEL_AUTOSAVE));

    let content_frame = panel.contentRectForFrameRect(panel.frame());
    let (w, hgt) = (content_frame.size.width, content_frame.size.height);
    let fx = NSVisualEffectView::initWithFrame(NSVisualEffectView::alloc(mtm), rect(0.0, 0.0, w, hgt));
    fx.setMaterial(NSVisualEffectMaterial::WindowBackground);
    fx.setBlendingMode(NSVisualEffectBlendingMode::BehindWindow);
    fx.setState(NSVisualEffectState::Active);
    panel.setContentView(Some(&fx));
    // Everything sits in `zoom`, whose bounds are its frame divided by the
    // interface size: App::apply_scale lays it out (no autoresizing here).
    let zoom = NSView::initWithFrame(NSView::alloc(mtm), rect(0.0, 0.0, w, hgt));
    zoom.setAutoresizesSubviews(false);
    fx.addSubview(&zoom);

    // ---- top bar
    let top = NSView::initWithFrame(NSView::alloc(mtm), rect(0.0, hgt - TOP, w, TOP));
    top.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewMinYMargin);
    // App::layout_top places its controls (autoresizing is lossy when narrow).
    top.setAutoresizesSubviews(false);
    let cy = (TOP - 24.0) / 2.0;
    let close = icon_button(mtm, rect(8.0, cy, 26.0, 24.0), "xmark", "Close (Esc)", h, sel!(closePanel:));
    top.addSubview(&close);
    let app_popup = NSPopUpButton::initWithFrame_pullsDown(NSPopUpButton::alloc(mtm), rect(38.0, cy, 176.0, 24.0), false);
    unsafe {
        app_popup.setTarget(Some(h));
        app_popup.setAction(Some(sel!(appFilterChanged:)));
    }
    app_popup.setFont(Some(&NSFont::systemFontOfSize(12.0)));
    app_popup.setFocusRingType(NSFocusRingType::None);
    top.addSubview(&app_popup);
    let star_btn = icon_button(mtm, rect(218.0, cy, 28.0, 24.0), "star", "Starred only (S)", h, sel!(toggleStarFilter:));
    top.addSubview(&star_btn);
    let search = NSSearchField::initWithFrame(NSSearchField::alloc(mtm), rect(252.0, cy, w - 252.0 - 72.0, 24.0));
    search.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable);
    search.setPlaceholderString(Some(&ns("Search  (⌘F)")));
    unsafe { search.setDelegate(Some(ProtocolObject::from_ref(&*a.handler))) };
    unsafe {
        search.setTarget(Some(h));
        search.setAction(Some(sel!(searchChanged:)));
    }
    search.setSendsSearchStringImmediately(true);
    search.setFocusRingType(NSFocusRingType::None);
    top.addSubview(&search);
    let preview_btn = icon_button(mtm, rect(w - 66.0, cy, 28.0, 24.0), "sidebar.right", "Hide preview (←)", h, sel!(togglePreview:));
    preview_btn.setAutoresizingMask(NSAutoresizingMaskOptions::ViewMinXMargin);
    top.addSubview(&preview_btn);
    let pin_btn = icon_button(mtm, rect(w - 36.0, cy, 28.0, 24.0), "pin", "Pin window open (⌘P)", h, sel!(togglePin:));
    pin_btn.setAutoresizingMask(NSAutoresizingMaskOptions::ViewMinXMargin);
    top.addSubview(&pin_btn);
    zoom.addSubview(&top);

    // ---- split: list | preview
    let split = NSSplitView::initWithFrame(NSSplitView::alloc(mtm), rect(0.0, BOTTOM, w, hgt - TOP - BOTTOM));
    split.setVertical(true);
    split.setDividerStyle(NSSplitViewDividerStyle::Thin);
    split.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewHeightSizable);
    let left_w = (w * 0.48).round();
    let list_h = hgt - TOP - BOTTOM;

    let scroll = NSScrollView::initWithFrame(NSScrollView::alloc(mtm), rect(0.0, 0.0, left_w, list_h));
    scroll.setHasVerticalScroller(true);
    scroll.setAutohidesScrollers(true);
    scroll.setDrawsBackground(false);
    scroll.setBorderType(NSBorderType::NoBorder);
    let table: Retained<ClipTable> = unsafe { msg_send![ClipTable::alloc(mtm), initWithFrame: rect(0.0, 0.0, left_w, list_h)] };
    let col = NSTableColumn::initWithIdentifier(NSTableColumn::alloc(mtm), &ns("clip"));
    col.setWidth(left_w);
    col.setResizingMask(NSTableColumnResizingOptions::AutoresizingMask);
    table.addTableColumn(&col);
    table.setHeaderView(None);
    table.setStyle(NSTableViewStyle::Plain);
    table.setRowHeight(a.settings.borrow().row_height);
    table.setIntercellSpacing(NSSize::new(0.0, 1.0));
    table.setGridStyleMask(NSTableViewGridLineStyle::SolidHorizontalGridLineMask);
    table.setGridColor(&NSColor::separatorColor());
    table.setBackgroundColor(&NSColor::clearColor());
    table.setColumnAutoresizingStyle(NSTableViewColumnAutoresizingStyle::FirstColumnOnlyAutoresizingStyle);
    table.setAllowsEmptySelection(true);
    table.setAllowsMultipleSelection(false);
    table.setFocusRingType(NSFocusRingType::None);
    unsafe {
        table.setDataSource(Some(ProtocolObject::from_ref(&*a.handler)));
        table.setDelegate(Some(ProtocolObject::from_ref(&*a.handler)));
        table.setTarget(Some(h));
        table.setDoubleAction(Some(sel!(rowDoubleClicked:)));
    }
    scroll.setDocumentView(Some(&table));
    split.addSubview(&scroll);

    let preview = NSView::initWithFrame(NSView::alloc(mtm), rect(0.0, 0.0, w - left_w - 1.0, list_h));
    let pw = w - left_w - 1.0;
    let meta_h = 96.0;
    let meta = NSTextField::wrappingLabelWithString(&ns(""), mtm);
    meta.setFrame(rect(10.0, 4.0, pw - 20.0, meta_h - 8.0));
    meta.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewMaxYMargin);
    meta.setFont(Some(&NSFont::systemFontOfSize(11.0)));
    meta.setTextColor(Some(&NSColor::secondaryLabelColor()));
    meta.setSelectable(true);
    preview.addSubview(&meta);
    let sep = objc2_app_kit::NSBox::initWithFrame(objc2_app_kit::NSBox::alloc(mtm), rect(0.0, meta_h, pw, 1.0));
    sep.setBoxType(objc2_app_kit::NSBoxType::Separator);
    sep.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewMaxYMargin);
    preview.addSubview(&sep);
    let area = rect(0.0, meta_h + 1.0, pw, list_h - meta_h - 1.0);
    let image_view = NSImageView::initWithFrame(NSImageView::alloc(mtm), rect(area.origin.x + 8.0, area.origin.y + 8.0, area.size.width - 16.0, area.size.height - 16.0));
    image_view.setImageScaling(NSImageScaling::ScaleProportionallyDown);
    image_view.setImageAlignment(NSImageAlignment::AlignCenter);
    image_view.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewHeightSizable);
    image_view.setHidden(true);
    preview.addSubview(&image_view);
    let text_scroll = NSTextView::scrollableTextView(mtm);
    text_scroll.setFrame(area);
    text_scroll.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewHeightSizable);
    text_scroll.setDrawsBackground(false);
    text_scroll.setBorderType(NSBorderType::NoBorder);
    let text_view: Retained<NSTextView> = text_scroll
        .documentView()
        .and_then(|v| v.downcast::<NSTextView>().ok())
        .expect("scrollableTextView has a text view");
    text_view.setEditable(false);
    text_view.setSelectable(true);
    text_view.setRichText(true);
    text_view.setDrawsBackground(false);
    text_view.setTextContainerInset(NSSize::new(8.0, 8.0));
    preview.addSubview(&text_scroll);
    split.addSubview(&preview);
    split.setAutosaveName(Some(&ns("cliprust.split")));
    split.adjustSubviews();
    zoom.addSubview(&split);

    // ---- bottom bar
    let bottom = NSView::initWithFrame(NSView::alloc(mtm), rect(0.0, 0.0, w, BOTTOM));
    bottom.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewMaxYMargin);
    let gear = NSPopUpButton::initWithFrame_pullsDown(NSPopUpButton::alloc(mtm), rect(6.0, 2.0, 44.0, 24.0), true);
    gear.setBordered(false);
    gear.setMenu(Some(&gear_menu(a, true)));
    gear.setFocusRingType(NSFocusRingType::None);
    bottom.addSubview(&gear);
    let date_label = label(mtm, rect(60.0, 6.0, w - 120.0, 16.0), 12.0);
    date_label.setAlignment(NSTextAlignment::Center);
    date_label.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable);
    bottom.addSubview(&date_label);
    let count_label = label(mtm, rect(w - 110.0, 7.0, 100.0, 14.0), 10.0);
    count_label.setAlignment(NSTextAlignment::Right);
    count_label.setTextColor(Some(&NSColor::tertiaryLabelColor()));
    count_label.setAutoresizingMask(NSAutoresizingMaskOptions::ViewMinXMargin);
    bottom.addSubview(&count_label);
    zoom.addSubview(&bottom);

    // ---- status item
    let status_item = NSStatusBar::systemStatusBar().statusItemWithLength(-1.0);
    if let Some(btn) = status_item.button(mtm) {
        btn.setImage(status_image().as_deref());
        unsafe {
            btn.setTarget(Some(h));
            btn.setAction(Some(sel!(statusClicked:)));
        }
        btn.sendActionOn(NSEventMask::LeftMouseUp | NSEventMask::RightMouseUp);
    }
    let status_menu = gear_menu(a, false);

    // Initial divider position when nothing was autosaved yet.
    let saved = objc2_foundation::NSUserDefaults::standardUserDefaults()
        .objectForKey(&ns("NSSplitView Subview Frames cliprust.split"))
        .is_some();
    if !saved {
        split.setPosition_ofDividerAtIndex(left_w, 0);
    }

    Ui {
        panel,
        table,
        search,
        app_popup,
        star_btn,
        pin_btn,
        preview_btn,
        split,
        list: scroll,
        preview,
        zoom,
        top_bar: top,
        bottom_bar: bottom,
        date_label,
        count_label,
        text_scroll,
        text_view,
        image_view,
        meta,
        status_item,
        status_menu,
    }
}

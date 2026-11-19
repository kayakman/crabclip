//! Custom drawing of a history row: source app icon, then the clip (text lines,
//! image thumbnail cropped to the row, or file icon + names), then the star.

use super::app;
use super::classes::{PillMenu, RowView, TagStrip};
use crate::db::Kind;
use objc2::DefinedClass;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_app_kit::{
    NSBezierPath, NSColor, NSCompositingOperation, NSFont, NSFontAttributeName,
    NSForegroundColorAttributeName, NSGraphicsContext, NSImage, NSImageSymbolConfiguration,
    NSAttributedStringNSExtendedStringDrawing, NSAttributedStringNSStringDrawing, NSLineBreakMode,
    NSMutableParagraphStyle, NSParagraphStyleAttributeName, NSStringDrawingOptions,
};
use objc2_foundation::{NSAttributedString, NSDictionary, NSPoint, NSRect, NSSize, NSString};

const ICON: f64 = 30.0;
const LEFT: f64 = 50.0;
const RIGHT: f64 = 34.0;

pub fn star_rect(b: NSRect) -> NSRect {
    NSRect::new(NSPoint::new(b.size.width - 30.0, (b.size.height - 24.0) / 2.0), NSSize::new(26.0, 24.0))
}

pub fn tinted_symbol(name: &str, color: &NSColor, size: f64) -> Option<Retained<NSImage>> {
    let img = super::symbol(name)?;
    let cfg = NSImageSymbolConfiguration::configurationWithPointSize_weight(size, 0.0);
    let tint = NSImageSymbolConfiguration::configurationWithHierarchicalColor(color);
    let cfg = cfg.configurationByApplyingConfiguration(&tint);
    img.imageWithSymbolConfiguration(&cfg)
}

fn draw_image(img: &NSImage, r: NSRect) {
    unsafe {
        img.drawInRect_fromRect_operation_fraction_respectFlipped_hints(
            r,
            NSRect::ZERO,
            NSCompositingOperation::SourceOver,
            1.0,
            true,
            None,
        )
    };
}

pub fn attrs(font: &NSFont, color: &NSColor, wrap: bool) -> objc2::rc::Retained<NSDictionary<NSString, AnyObject>> {
    let para = NSMutableParagraphStyle::new();
    para.setLineBreakMode(if wrap { NSLineBreakMode::ByWordWrapping } else { NSLineBreakMode::ByTruncatingTail });
    unsafe {
        NSDictionary::from_slices(
            &[NSFontAttributeName, NSForegroundColorAttributeName, NSParagraphStyleAttributeName],
            &[font as &AnyObject, color as &AnyObject, &*para as &AnyObject],
        )
    }
}

fn draw_text(text: &str, r: NSRect, font: &NSFont, color: &NSColor, wrap: bool) {
    let a = attrs(font, color, wrap);
    let s = unsafe { NSAttributedString::new_with_attributes(&NSString::from_str(text), &a) };
    let opts = NSStringDrawingOptions::UsesLineFragmentOrigin | NSStringDrawingOptions::TruncatesLastVisibleLine;
    // Vertically center text that is shorter than the row.
    let need = s.boundingRectWithSize_options_context(r.size, opts, None).size.height.ceil();
    let mut r = r;
    if need < r.size.height {
        r.origin.y += ((r.size.height - need) / 2.0).floor();
        r.size.height = need + 1.0;
    }
    s.drawWithRect_options_context(r, opts, None);
}

// ------------------------------------------------------------------ pills

fn dark() -> bool {
    let name = objc2_app_kit::NSAppearance::currentDrawingAppearance().name().to_string();
    name.contains("Dark")
}

fn srgb(r: u8, g: u8, b: u8, a: f64) -> Retained<NSColor> {
    NSColor::colorWithSRGBRed_green_blue_alpha(r as f64 / 255.0, g as f64 / 255.0, b as f64 / 255.0, a)
}

/// Outlined blue tag look: (fill, border, text, count bubble).
fn pill_colors() -> [Retained<NSColor>; 4] {
    if dark() {
        [srgb(70, 130, 230, 0.18), srgb(110, 160, 240, 0.55), srgb(160, 196, 255, 1.0), srgb(110, 160, 240, 0.28)]
    } else {
        [srgb(238, 244, 253, 1.0), srgb(160, 190, 236, 1.0), srgb(52, 104, 196, 1.0), srgb(214, 228, 250, 1.0)]
    }
}

fn pill_text(label: &str, h: f64, color: &NSColor) -> Retained<NSAttributedString> {
    let font = NSFont::systemFontOfSize_weight((h * 0.6).round(), 0.23); // medium
    let a = attrs(&font, color, false);
    unsafe { NSAttributedString::new_with_attributes(&NSString::from_str(label), &a) }
}

fn count_width(count: Option<i64>, h: f64) -> f64 {
    match count {
        Some(n) => pill_text(&n.to_string(), h, &NSColor::labelColor()).size().width.ceil() + h * 0.7 + h * 0.3,
        None => 0.0,
    }
}

pub fn pill_width(label: &str, lock: bool, h: f64) -> f64 {
    pill_width_n(label, lock, h, None)
}

fn pill_width_n(label: &str, lock: bool, h: f64, count: Option<i64>) -> f64 {
    let tw = pill_text(label, h, &NSColor::labelColor()).size().width.ceil();
    let pad = h * 0.55;
    tw + pad * 2.0 + if lock { h * 0.65 } else { 0.0 } + count_width(count, h) - if count.is_some() { pad * 0.5 } else { 0.0 }
}

/// A tag pill with its top-left at `at`; returns its width.
pub fn pill(label: &str, lock: bool, at: NSPoint, h: f64) -> f64 {
    pill_n(label, lock, at, h, None)
}

/// A tag pill, optionally with a count bubble inside on the right.
fn pill_n(label: &str, lock: bool, at: NSPoint, h: f64, count: Option<i64>) -> f64 {
    let [fill, border, text, bubble] = pill_colors();
    let w = pill_width_n(label, lock, h, count);
    let r = NSRect::new(NSPoint::new(at.x + 0.5, at.y + 0.5), NSSize::new(w - 1.0, h - 1.0));
    let path = NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(r, (h - 1.0) / 2.0, (h - 1.0) / 2.0);
    fill.setFill();
    path.fill();
    border.setStroke();
    path.setLineWidth(1.0);
    path.stroke();
    let mut x = at.x + h * 0.55;
    if lock {
        if let Some(img) = tinted_symbol("lock.fill", &text, (h * 0.48).round()) {
            let sz = img.size();
            draw_image(&img, NSRect::new(NSPoint::new(x, at.y + (h - sz.height) / 2.0), sz));
        }
        x += h * 0.65;
    }
    let t = pill_text(label, h, &text);
    let ts = t.size();
    t.drawAtPoint(NSPoint::new(x, at.y + (h - ts.height) / 2.0));
    if let Some(n) = count {
        let c = pill_text(&n.to_string(), h, &text);
        let cs = c.size();
        let bh = h - 6.0;
        let bw = cs.width.ceil() + h * 0.7;
        let bx = at.x + w - 3.0 - bw;
        let br = NSRect::new(NSPoint::new(bx, at.y + 3.0), NSSize::new(bw, bh));
        bubble.setFill();
        NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(br, bh / 2.0, bh / 2.0).fill();
        c.drawAtPoint(NSPoint::new(bx + (bw - cs.width) / 2.0, at.y + (h - cs.height) / 2.0));
    }
    w
}

pub fn tag_strip(v: &TagStrip) {
    let b = v.bounds();
    let tags = v.ivars().tags.borrow();
    if tags.is_empty() {
        let f = NSFont::systemFontOfSize(12.0);
        draw_text("Tags (⌘T) · “password” encrypts the clip", b, &f, &NSColor::placeholderTextColor(), false);
        return;
    }
    let h = 20.0;
    let mut x = 0.0;
    for t in tags.iter() {
        let lock = t == crate::db::PASSWORD;
        if x + pill_width(t, lock, h) > b.size.width {
            break;
        }
        x += pill(t, lock, NSPoint::new(x, (b.size.height - h) / 2.0), h) + 6.0;
    }
}

const MENU_PAD: f64 = 6.0;
const MENU_ROW: f64 = 30.0;
const MENU_PILL: f64 = 22.0;

pub fn pill_menu_size(items: &[(String, i64)]) -> NSSize {
    let widest = items
        .iter()
        .map(|(t, n)| pill_width_n(t, t == crate::db::PASSWORD, MENU_PILL, Some(*n)))
        .fold(0.0, f64::max);
    NSSize::new((widest + 56.0).max(160.0), MENU_PAD * 2.0 + MENU_ROW * items.len() as f64)
}

pub fn pill_menu_item_at(y: f64) -> Option<usize> {
    if y < MENU_PAD { None } else { Some(((y - MENU_PAD) / MENU_ROW) as usize) }
}

pub fn pill_menu(v: &PillMenu) {
    let b = v.bounds();
    let bg = NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(b, 8.0, 8.0);
    NSColor::windowBackgroundColor().setFill();
    bg.fill();
    NSColor::separatorColor().setStroke();
    bg.stroke();
    let small = NSFont::systemFontOfSize(11.0);
    for (i, (name, count)) in v.ivars().items.borrow().iter().enumerate() {
        let y = MENU_PAD + i as f64 * MENU_ROW;
        pill_n(name, name == crate::db::PASSWORD, NSPoint::new(10.0, y + (MENU_ROW - MENU_PILL) / 2.0), MENU_PILL, Some(*count));
        if i == 0 {
            let r = NSRect::new(NSPoint::new(b.size.width - 34.0, y), NSSize::new(24.0, MENU_ROW));
            draw_text("⇥", r, &small, &NSColor::tertiaryLabelColor(), false);
        }
    }
}

pub fn row(v: &RowView) {
    let id = v.ivars().clip.get();
    let a = app();
    let Some(d) = a.row_data(id) else { return };
    let b = v.bounds();
    let sel = v.ivars().selected.get();
    let (w, h) = (b.size.width, b.size.height);

    let icon = a.app_icon(&d.info);
    draw_image(&icon, NSRect::new(NSPoint::new(10.0, (h - ICON) / 2.0), NSSize::new(ICON, ICON)));

    let content = NSRect::new(NSPoint::new(LEFT, 5.0), NSSize::new((w - LEFT - RIGHT).max(10.0), h - 10.0));
    let fg = if sel { NSColor::alternateSelectedControlTextColor() } else { NSColor::labelColor() };
    let font_size = a.settings.borrow().font_size;
    let font = NSFont::systemFontOfSize(font_size);
    let sealed = d.info.tags.iter().any(|t| t == crate::db::PASSWORD);

    match d.info.kind {
        _ if sealed => {
            draw_text(&d.info.preview, content, &font, &fg, false);
        }
        Kind::Image => {
            if let Some(img) = &d.thumb {
                let ts = img.size();
                if ts.width > 0.0 && ts.height > 0.0 {
                    let natural = if d.info.img_w > 0 { d.info.img_w as f64 / d.info.img_scale } else { ts.width / 2.0 };
                    let dw = content.size.width.min(natural).max(1.0);
                    let k = dw / ts.width;
                    let dh = ts.height * k;
                    let y = if dh < content.size.height {
                        content.origin.y + (content.size.height - dh) / 2.0
                    } else {
                        content.origin.y
                    };
                    NSGraphicsContext::saveGraphicsState_class();
                    NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(content, 3.0, 3.0).addClip();
                    draw_image(img, NSRect::new(NSPoint::new(content.origin.x, y), NSSize::new(dw, dh)));
                    NSGraphicsContext::restoreGraphicsState_class();
                }
            } else {
                draw_text("Image", content, &font, &fg, false);
            }
        }
        Kind::File => {
            let path = d.info.preview.lines().next().unwrap_or("").to_string();
            let first = path.rsplit('/').next().unwrap_or(&path).to_string();
            let fi = a.file_icon(&path);
            let isz = (content.size.height - 4.0).min(40.0);
            draw_image(&fi, NSRect::new(NSPoint::new(content.origin.x, content.origin.y + (content.size.height - isz) / 2.0), NSSize::new(isz, isz)));
            let n = d.info.preview.lines().count();
            let dir = path.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
            let home = std::env::var("HOME").unwrap_or_default();
            let dir = match dir.strip_prefix(&home) {
                Some(rest) if !home.is_empty() => format!("~{rest}"),
                _ => dir.to_string(),
            };
            let label = if n > 1 { format!("{first}\n+ {} more", n - 1) } else { format!("{first}\n{dir}") };
            let tr = NSRect::new(
                NSPoint::new(content.origin.x + isz + 8.0, content.origin.y),
                NSSize::new(content.size.width - isz - 8.0, content.size.height),
            );
            draw_text(&label, tr, &font, &fg, true);
        }
        Kind::Url => {
            let c = if sel { fg.clone() } else { NSColor::linkColor() };
            draw_text(d.info.preview.trim(), content, &font, &c, true);
        }
        _ => {
            draw_text(d.info.preview.trim(), content, &font, &fg, true);
        }
    }

    // Tag pills, bottom right of the clip, on a backing so text doesn't show through.
    let ph = (16.0 * font_size / 13.0).round();
    let right = content.origin.x + content.size.width;
    let mut fits = Vec::new();
    let mut x = right;
    for t in d.info.tags.iter() {
        let lock = t == crate::db::PASSWORD;
        let pw = pill_width(t, lock, ph);
        if x - pw < content.origin.x + 40.0 {
            break;
        }
        x -= pw;
        fits.push((t, lock, x));
        x -= 4.0;
    }
    if !fits.is_empty() {
        let y = h - 5.0 - ph;
        let back = NSRect::new(NSPoint::new(x - 2.0, y - 3.0), NSSize::new(right - x + 6.0, ph + 6.0));
        let bg = if sel { NSColor::selectedContentBackgroundColor() } else { NSColor::windowBackgroundColor() };
        bg.setFill();
        NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(back, (ph + 6.0) / 2.0, (ph + 6.0) / 2.0).fill();
        for (t, lock, px) in fits {
            pill(t, lock, NSPoint::new(px, y), ph);
        }
    }

    let star = if d.info.starred {
        tinted_symbol("star.fill", &NSColor::systemYellowColor(), 15.0)
    } else if sel {
        tinted_symbol("star", &NSColor::alternateSelectedControlTextColor(), 15.0)
    } else {
        tinted_symbol("star", &NSColor::tertiaryLabelColor(), 15.0)
    };
    if let Some(s) = star {
        let sr = star_rect(b);
        let sz = s.size();
        draw_image(
            &s,
            NSRect::new(
                NSPoint::new(sr.origin.x + (sr.size.width - sz.width) / 2.0, sr.origin.y + (sr.size.height - sz.height) / 2.0),
                sz,
            ),
        );
    }
}

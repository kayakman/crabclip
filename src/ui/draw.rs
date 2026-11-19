//! Custom drawing of a history row: source app icon, then the clip (text lines,
//! image thumbnail cropped to the row, or file icon + names), then the star.

use super::app;
use super::classes::{PillMenu, RowView, TagStrip};
use crate::db::Kind;
use objc2::{AnyThread, DefinedClass};
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_app_kit::{
    NSBezierPath, NSColor, NSCompositingOperation, NSFont, NSFontAttributeName,
    NSForegroundColorAttributeName, NSGraphicsContext, NSImage, NSImageSymbolConfiguration,
    NSAttributedStringNSExtendedStringDrawing, NSAttributedStringNSStringDrawing, NSGradient, NSLineBreakMode,
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

fn pill_text(label: &str, h: f64) -> Retained<NSAttributedString> {
    let font = NSFont::boldSystemFontOfSize((h * 0.6).round());
    let a = attrs(&font, &NSColor::whiteColor(), false);
    unsafe { NSAttributedString::new_with_attributes(&NSString::from_str(label), &a) }
}

pub fn pill_width(label: &str, lock: bool, h: f64) -> f64 {
    let tw = pill_text(label, h).size().width.ceil();
    tw + h * 0.9 + if lock { h * 0.7 } else { 0.0 }
}

/// A glassy blue tag pill with its top-left at `at`; returns its width.
pub fn pill(label: &str, lock: bool, at: NSPoint, h: f64) -> f64 {
    let w = pill_width(label, lock, h);
    let r = NSRect::new(at, NSSize::new(w, h));
    let path = NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(r, h / 2.0, h / 2.0);
    let top = NSColor::colorWithSRGBRed_green_blue_alpha(0.45, 0.70, 1.0, 0.95);
    let bottom = NSColor::colorWithSRGBRed_green_blue_alpha(0.05, 0.38, 0.92, 0.95);
    if let Some(g) = NSGradient::initWithStartingColor_endingColor(NSGradient::alloc(), &top, &bottom) {
        g.drawInBezierPath_angle(&path, 90.0);
    }
    // Gloss over the top half, and a light rim.
    let gr = NSRect::new(NSPoint::new(at.x + 1.5, at.y + 1.0), NSSize::new(w - 3.0, h * 0.46));
    let gloss = NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(gr, h * 0.23, h * 0.23);
    NSColor::colorWithWhite_alpha(1.0, 0.22).setFill();
    gloss.fill();
    NSColor::colorWithWhite_alpha(1.0, 0.5).setStroke();
    path.setLineWidth(0.6);
    path.stroke();
    let mut x = at.x + h * 0.45;
    if lock {
        if let Some(img) = tinted_symbol("lock.fill", &NSColor::whiteColor(), (h * 0.5).round()) {
            let sz = img.size();
            draw_image(&img, NSRect::new(NSPoint::new(x, at.y + (h - sz.height) / 2.0), sz));
        }
        x += h * 0.7;
    }
    let t = pill_text(label, h);
    let th = t.size().height;
    t.drawAtPoint(NSPoint::new(x, at.y + (h - th) / 2.0));
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
    let h = 18.0;
    let mut x = 0.0;
    for t in tags.iter() {
        let lock = t == crate::db::PASSWORD;
        if x + pill_width(t, lock, h) > b.size.width {
            break;
        }
        x += pill(t, lock, NSPoint::new(x, (b.size.height - h) / 2.0), h) + 5.0;
    }
}

const MENU_PAD: f64 = 6.0;
const MENU_ROW: f64 = 28.0;

pub fn pill_menu_size(items: &[(String, i64)]) -> NSSize {
    let widest = items.iter().map(|(t, _)| pill_width(t, t == crate::db::PASSWORD, 18.0)).fold(0.0, f64::max);
    NSSize::new((widest + 90.0).max(200.0), MENU_PAD * 2.0 + MENU_ROW * items.len() as f64)
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
        pill(name, name == crate::db::PASSWORD, NSPoint::new(10.0, y + (MENU_ROW - 18.0) / 2.0), 18.0);
        let hint = if i == 0 { format!("{count}   ⇥") } else { count.to_string() };
        let r = NSRect::new(NSPoint::new(b.size.width - 70.0, y), NSSize::new(60.0, MENU_ROW));
        let para = NSMutableParagraphStyle::new();
        para.setAlignment(objc2_app_kit::NSTextAlignment::Right);
        let a = unsafe {
            NSDictionary::from_slices(
                &[NSFontAttributeName, NSForegroundColorAttributeName, NSParagraphStyleAttributeName],
                &[&*small as &AnyObject, &*NSColor::secondaryLabelColor() as &AnyObject, &*para as &AnyObject],
            )
        };
        let s = unsafe { NSAttributedString::new_with_attributes(&NSString::from_str(&hint), &a) };
        let th = s.size().height;
        s.drawInRect(NSRect::new(NSPoint::new(r.origin.x, y + (MENU_ROW - th) / 2.0), NSSize::new(r.size.width, th)));
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

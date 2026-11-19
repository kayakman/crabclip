//! Custom drawing of a history row: source app icon, then the clip (text lines,
//! image thumbnail cropped to the row, or file icon + names), then the star.

use super::app;
use super::classes::RowView;
use crate::db::Kind;
use objc2::DefinedClass;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_app_kit::{
    NSBezierPath, NSColor, NSCompositingOperation, NSFont, NSFontAttributeName,
    NSForegroundColorAttributeName, NSGraphicsContext, NSImage, NSImageSymbolConfiguration,
    NSAttributedStringNSExtendedStringDrawing, NSLineBreakMode, NSMutableParagraphStyle, NSParagraphStyleAttributeName, NSStringDrawingOptions,
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
    let font = NSFont::systemFontOfSize(a.settings.borrow().font_size);

    match d.info.kind {
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

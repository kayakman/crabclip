//! ImageIO / CoreGraphics helpers (thread-safe): dimensions, list thumbnails and
//! optional downscaling. CF types are passed around as raw pointers; Foundation
//! objects are toll-free bridged where a CFDictionary/CFData/CFString is expected.

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_foundation::{NSData, NSDictionary, NSMutableData, NSNumber, NSString};
use std::ffi::c_void;

type CFRef = *const c_void;

#[repr(C)]
#[derive(Clone, Copy)]
struct CGRect {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

#[link(name = "ImageIO", kind = "framework")]
unsafe extern "C" {
    fn CGImageSourceCreateWithData(data: CFRef, options: CFRef) -> CFRef;
    fn CGImageSourceCopyPropertiesAtIndex(src: CFRef, index: usize, options: CFRef) -> CFRef;
    fn CGImageSourceCreateImageAtIndex(src: CFRef, index: usize, options: CFRef) -> CFRef;
    fn CGImageSourceCreateThumbnailAtIndex(src: CFRef, index: usize, options: CFRef) -> CFRef;
    fn CGImageDestinationCreateWithData(data: CFRef, ty: CFRef, count: usize, options: CFRef) -> CFRef;
    fn CGImageDestinationAddImage(dest: CFRef, image: CFRef, props: CFRef);
    fn CGImageDestinationFinalize(dest: CFRef) -> bool;
    static kCGImagePropertyPixelWidth: CFRef;
    static kCGImagePropertyPixelHeight: CFRef;
    static kCGImagePropertyDPIWidth: CFRef;
    static kCGImagePropertyOrientation: CFRef;
    static kCGImageSourceThumbnailMaxPixelSize: CFRef;
    static kCGImageSourceCreateThumbnailFromImageAlways: CFRef;
    static kCGImageSourceCreateThumbnailWithTransform: CFRef;
    static kCGImageDestinationLossyCompressionQuality: CFRef;
}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGImageGetWidth(img: CFRef) -> usize;
    fn CGImageGetHeight(img: CFRef) -> usize;
    fn CGImageCreateWithImageInRect(img: CFRef, rect: CGRect) -> CFRef;
    fn CGColorSpaceCreateDeviceRGB() -> CFRef;
    fn CGBitmapContextCreate(
        data: *mut c_void,
        w: usize,
        h: usize,
        bpc: usize,
        bpr: usize,
        space: CFRef,
        info: u32,
    ) -> CFRef;
    fn CGBitmapContextCreateImage(ctx: CFRef) -> CFRef;
    fn CGContextSetRGBFillColor(ctx: CFRef, r: f64, g: f64, b: f64, a: f64);
    fn CGContextFillRect(ctx: CFRef, rect: CGRect);
    fn CGContextSetInterpolationQuality(ctx: CFRef, q: i32);
    fn CGContextDrawImage(ctx: CFRef, rect: CGRect, img: CFRef);
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFRelease(cf: CFRef);
}

/// Owned CF object, released on drop.
struct Cf(CFRef);
impl Drop for Cf {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { CFRelease(self.0) }
        }
    }
}
impl Cf {
    fn new(p: CFRef) -> Option<Cf> {
        if p.is_null() { None } else { Some(Cf(p)) }
    }
}

unsafe fn key(k: CFRef) -> &'static NSString {
    unsafe { &*(k as *const NSString) }
}

fn ns_ptr<T: objc2::Message>(o: &T) -> CFRef {
    o as *const T as CFRef
}

fn source(data: &[u8]) -> Option<(Cf, Retained<NSData>)> {
    let nsdata = NSData::with_bytes(data);
    let src = unsafe { CGImageSourceCreateWithData(ns_ptr(&*nsdata), std::ptr::null()) };
    Some((Cf::new(src)?, nsdata))
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Info {
    pub width: i64,
    pub height: i64,
    /// Pixels per point (2 for Retina screenshots with 144 dpi metadata).
    pub scale: f64,
}

pub fn info(data: &[u8]) -> Option<Info> {
    let (src, _keep) = source(data)?;
    unsafe {
        let props = Cf::new(CGImageSourceCopyPropertiesAtIndex(src.0, 0, std::ptr::null()))?;
        let dict: &NSDictionary<NSString, AnyObject> = &*(props.0 as *const NSDictionary<NSString, AnyObject>);
        let num = |k: CFRef| -> Option<f64> {
            dict.objectForKey(key(k))
                .and_then(|o| o.downcast::<NSNumber>().ok())
                .map(|n| n.doubleValue())
        };
        let mut w = num(kCGImagePropertyPixelWidth)? as i64;
        let mut h = num(kCGImagePropertyPixelHeight)? as i64;
        // EXIF orientations 5-8 are rotated 90 degrees.
        if num(kCGImagePropertyOrientation).is_some_and(|o| o >= 5.0) {
            std::mem::swap(&mut w, &mut h);
        }
        let dpi = num(kCGImagePropertyDPIWidth).unwrap_or(72.0);
        let scale = if dpi >= 140.0 { 2.0 } else { 1.0 };
        Some(Info { width: w, height: h, scale })
    }
}

fn opts(pairs: &[(CFRef, Retained<AnyObject>)]) -> Retained<NSDictionary<NSString, AnyObject>> {
    let keys: Vec<&NSString> = pairs.iter().map(|(k, _)| unsafe { key(*k) }).collect();
    let vals: Vec<&AnyObject> = pairs.iter().map(|(_, v)| &**v).collect();
    NSDictionary::from_slices(&keys, &vals)
}

fn num_obj(v: f64) -> Retained<AnyObject> {
    Retained::into_super(Retained::into_super(Retained::into_super(NSNumber::new_f64(v))))
}

fn bool_obj(v: bool) -> Retained<AnyObject> {
    Retained::into_super(Retained::into_super(Retained::into_super(NSNumber::new_bool(v))))
}

fn full_image(src: &Cf) -> Option<Cf> {
    unsafe {
        let o = opts(&[(kCGImageSourceCreateThumbnailWithTransform, bool_obj(true))]);
        Cf::new(CGImageSourceCreateImageAtIndex(src.0, 0, ns_ptr(&*o)))
    }
}

fn encode(img: &Cf, uti: &str, quality: Option<f64>) -> Option<Vec<u8>> {
    unsafe {
        let out = NSMutableData::new();
        let ty = NSString::from_str(uti);
        let dest = Cf::new(CGImageDestinationCreateWithData(ns_ptr(&*out), ns_ptr(&*ty), 1, std::ptr::null()))?;
        let props = quality.map(|q| opts(&[(kCGImageDestinationLossyCompressionQuality, num_obj(q))]));
        let props_ptr = props.as_ref().map(|p| ns_ptr(&**p)).unwrap_or(std::ptr::null());
        CGImageDestinationAddImage(dest.0, img.0, props_ptr);
        if !CGImageDestinationFinalize(dest.0) {
            return None;
        }
        Some(out.to_vec())
    }
}

/// List thumbnail: the image scaled to at most `max_w` px wide, cropped to the top
/// `max_h` px, flattened on white and JPEG encoded. ImageIO does the downscale
/// (and applies EXIF rotation), so huge images are never decoded at full size.
pub fn thumbnail(data: &[u8], max_w: usize, max_h: usize) -> Option<Vec<u8>> {
    let i = info(data)?;
    let (w, h) = (i.width.max(1) as f64, i.height.max(1) as f64);
    let scale = (max_w as f64 / w).min(1.0);
    // Longest side after scaling to the target width; very tall images are
    // capped (their top part is all the list shows anyway).
    let max_px = (w.max(h) * scale).ceil().clamp(1.0, 4096.0);
    let (src, _keep) = source(data)?;
    unsafe {
        let o = opts(&[
            (kCGImageSourceThumbnailMaxPixelSize, num_obj(max_px)),
            (kCGImageSourceCreateThumbnailFromImageAlways, bool_obj(true)),
            (kCGImageSourceCreateThumbnailWithTransform, bool_obj(true)),
        ]);
        let img = Cf::new(CGImageSourceCreateThumbnailAtIndex(src.0, 0, ns_ptr(&*o)))?;
        let (iw, ih) = (CGImageGetWidth(img.0), CGImageGetHeight(img.0));
        if iw == 0 || ih == 0 {
            return None;
        }
        let tw = iw.min(max_w);
        let k = tw as f64 / iw as f64;
        let th = ((ih as f64 * k).round() as usize).clamp(1, max_h);
        let src_h = ((th as f64 / k).round() as usize).clamp(1, ih);
        let cropped = Cf::new(CGImageCreateWithImageInRect(
            img.0,
            CGRect { x: 0.0, y: 0.0, w: iw as f64, h: src_h as f64 },
        ))?;
        let space = Cf::new(CGColorSpaceCreateDeviceRGB())?;
        // kCGImageAlphaPremultipliedLast = 1
        let ctx = Cf::new(CGBitmapContextCreate(std::ptr::null_mut(), tw, th, 8, 0, space.0, 1))?;
        CGContextSetInterpolationQuality(ctx.0, 3);
        let rect = CGRect { x: 0.0, y: 0.0, w: tw as f64, h: th as f64 };
        CGContextSetRGBFillColor(ctx.0, 1.0, 1.0, 1.0, 1.0);
        CGContextFillRect(ctx.0, rect);
        CGContextDrawImage(ctx.0, rect, cropped.0);
        let out = Cf::new(CGBitmapContextCreateImage(ctx.0))?;
        encode(&out, "public.jpeg", Some(0.82))
    }
}

/// Re-encode the image scaled so its longest side is `max_px`, in format `uti`.
pub fn scaled(data: &[u8], max_px: usize, uti: &str) -> Option<Vec<u8>> {
    let (src, _keep) = source(data)?;
    unsafe {
        let o = opts(&[
            (kCGImageSourceThumbnailMaxPixelSize, num_obj(max_px as f64)),
            (kCGImageSourceCreateThumbnailFromImageAlways, bool_obj(true)),
            (kCGImageSourceCreateThumbnailWithTransform, bool_obj(true)),
        ]);
        let img = Cf::new(CGImageSourceCreateThumbnailAtIndex(src.0, 0, ns_ptr(&*o)))?;
        let quality = if uti == "public.jpeg" || uti == "public.heic" { Some(0.92) } else { None };
        encode(&img, uti, quality)
    }
}

/// Convert any image to PNG (used for OCR input and app icons).
pub fn to_png(data: &[u8], max_px: Option<usize>) -> Option<Vec<u8>> {
    match max_px {
        Some(m) => scaled(data, m, "public.png"),
        None => {
            let (src, _keep) = source(data)?;
            let img = full_image(&src)?;
            encode(&img, "public.png", None)
        }
    }
}

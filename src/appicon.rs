//! Small PNG of an application's icon, kept in the database so history keeps its
//! icons even after an app is removed.

use objc2_app_kit::NSWorkspace;
use objc2_foundation::NSString;

pub fn png_for_path(path: &str) -> Option<Vec<u8>> {
    if !std::path::Path::new(path).exists() {
        return None;
    }
    objc2::rc::autoreleasepool(|_| {
        let icon = NSWorkspace::sharedWorkspace().iconForFile(&NSString::from_str(path));
        let tiff = icon.TIFFRepresentation()?;
        crate::imageio::to_png(&tiff.to_vec(), Some(64))
    })
}

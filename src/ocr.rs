//! Text recognition in images with Apple's Vision framework (runs off the main thread).

use objc2::rc::{Retained, autoreleasepool};
use objc2::runtime::{AnyClass, AnyObject};
use objc2::msg_send;
use objc2_foundation::{NSArray, NSData, NSDictionary, NSError, NSString};

#[link(name = "Vision", kind = "framework")]
unsafe extern "C" {}

pub fn recognize(image: &[u8]) -> Result<String, String> {
    autoreleasepool(|_| unsafe {
        let req_cls = AnyClass::get(c"VNRecognizeTextRequest").ok_or("Vision unavailable")?;
        let handler_cls = AnyClass::get(c"VNImageRequestHandler").ok_or("Vision unavailable")?;
        let req: Retained<AnyObject> = msg_send![req_cls, new];
        // VNRequestTextRecognitionLevelAccurate = 0
        let _: () = msg_send![&*req, setRecognitionLevel: 0isize];
        let _: () = msg_send![&*req, setUsesLanguageCorrection: true];
        let responds: bool = msg_send![&*req, respondsToSelector: objc2::sel!(setAutomaticallyDetectsLanguage:)];
        if responds {
            let _: () = msg_send![&*req, setAutomaticallyDetectsLanguage: true];
        }
        let data = NSData::with_bytes(image);
        let options = NSDictionary::<NSString, AnyObject>::new();
        let alloc: objc2::rc::Allocated<AnyObject> = msg_send![handler_cls, alloc];
        let handler: Retained<AnyObject> = msg_send![alloc, initWithData: &*data, options: &*options];
        let reqs = NSArray::from_slice(&[&*req]);
        let res: Result<(), Retained<NSError>> = msg_send![&*handler, performRequests: &*reqs, error: _];
        if let Err(e) = res {
            return Err(e.localizedDescription().to_string());
        }
        let results: Option<Retained<NSArray<AnyObject>>> = msg_send![&*req, results];
        let mut lines = Vec::new();
        for obs in results.iter().flat_map(|a| a.iter()) {
            let cands: Retained<NSArray<AnyObject>> = msg_send![&*obs, topCandidates: 1usize];
            if let Some(c) = cands.iter().next() {
                let s: Retained<NSString> = msg_send![&*c, string];
                lines.push(s.to_string());
            }
        }
        Ok(lines.join("\n"))
    })
}

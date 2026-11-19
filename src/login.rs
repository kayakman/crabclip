//! Launch at login through SMAppService: a LaunchAgent bundled in the app
//! (Contents/Library/LaunchAgents) with KeepAlive, so a crash restarts crabclip
//! but Quit (exit 0) does not. Falls back to the plain login item.

use objc2::msg_send;
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject};
use objc2_foundation::{NSError, NSString};

#[link(name = "ServiceManagement", kind = "framework")]
unsafe extern "C" {}

pub const AGENT_PLIST: &str = "com.rod.crabclip.agent.plist";

fn service(agent: bool) -> Option<Retained<AnyObject>> {
    let cls = AnyClass::get(c"SMAppService")?;
    unsafe {
        if agent {
            let name = NSString::from_str(AGENT_PLIST);
            msg_send![cls, agentServiceWithPlistName: &*name]
        } else {
            msg_send![cls, mainAppService]
        }
    }
}

/// 0 notRegistered, 1 enabled, 2 requiresApproval, 3 notFound
fn status(s: &AnyObject) -> isize {
    unsafe { msg_send![s, status] }
}

pub fn is_enabled() -> bool {
    [true, false].iter().any(|a| service(*a).is_some_and(|s| status(&s) == 1))
}

pub fn set_enabled(on: bool) -> Result<(), String> {
    let agent = service(true).ok_or("SMAppService unavailable")?;
    let main = service(false).ok_or("SMAppService unavailable")?;
    unsafe {
        if on {
            if status(&agent) == 1 {
                return Ok(());
            }
            let r: Result<(), Retained<NSError>> = msg_send![&*agent, registerAndReturnError: _];
            match r {
                Ok(()) => Ok(()),
                Err(e) => {
                    crate::log!("login: agent register failed: {}", e.localizedDescription());
                    let r: Result<(), Retained<NSError>> = msg_send![&*main, registerAndReturnError: _];
                    r.map_err(|e| e.localizedDescription().to_string())
                }
            }
        } else {
            for s in [agent, main] {
                if status(&s) == 1 || status(&s) == 2 {
                    let _: Result<(), Retained<NSError>> = msg_send![&*s, unregisterAndReturnError: _];
                }
            }
            Ok(())
        }
    }
}

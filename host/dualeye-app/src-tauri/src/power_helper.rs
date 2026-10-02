//! The root helper that reads the exact CPU power on macOS 27
//! (`dualeye-power-helper`, see `dualeye_core::sensors::power_helper`).
//!
//! It is a LaunchDaemon inside the bundle
//! (`Contents/Library/LaunchDaemons/com.dualeye.monitor.power.plist`, the
//! binary in `Contents/MacOS`), registered with `SMAppService`: the user
//! approves it once in System Settings > General > Login Items & Extensions,
//! and launchd keeps it running from then on, as root. Registering needs the
//! app signed by a team; an ad-hoc signed build reports `unavailable`.

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HelperState {
    /// Not macOS 13+, or not running from a bundle that carries the helper.
    Unavailable,
    Off,
    /// Registered; waiting for the user to allow it in System Settings.
    NeedsApproval,
    On,
}

#[derive(Debug, Clone, Serialize)]
pub struct HelperStatus {
    pub state: HelperState,
    /// The last registration error, for the UI.
    pub error: Option<String>,
}

#[cfg(target_os = "macos")]
mod imp {
    use objc2::rc::Retained;
    use objc2::runtime::{AnyClass, AnyObject};
    use objc2::msg_send;
    use objc2_foundation::{NSError, NSString};

    use super::HelperState;

    const PLIST: &str = "com.dualeye.monitor.power.plist";

    #[link(name = "ServiceManagement", kind = "framework")]
    unsafe extern "C" {}

    fn class() -> Option<&'static AnyClass> {
        AnyClass::get(c"SMAppService")
    }

    fn service() -> Option<Retained<AnyObject>> {
        let name = NSString::from_str(PLIST);
        unsafe { msg_send![class()?, daemonServiceWithPlistName: &*name] }
    }

    fn describe(e: &NSError) -> String {
        format!("{} (code {})", e.localizedDescription(), e.code())
    }

    pub fn state() -> HelperState {
        let Some(service) = service() else {
            return HelperState::Unavailable;
        };
        // SMAppServiceStatus: notRegistered, enabled, requiresApproval, notFound.
        let status: isize = unsafe { msg_send![&*service, status] };
        match status {
            0 => HelperState::Off,
            1 => HelperState::On,
            2 => HelperState::NeedsApproval,
            _ => HelperState::Unavailable,
        }
    }

    pub fn register() -> Result<(), String> {
        let service = service().ok_or("needs macOS 13 or newer")?;
        let done: Result<(), Retained<NSError>> = unsafe { msg_send![&*service, registerAndReturnError: _] };
        done.map_err(|e| describe(&e))
    }

    pub fn unregister() -> Result<(), String> {
        let service = service().ok_or("needs macOS 13 or newer")?;
        let done: Result<(), Retained<NSError>> = unsafe { msg_send![&*service, unregisterAndReturnError: _] };
        done.map_err(|e| describe(&e))
    }

    pub fn open_settings() {
        if let Some(class) = class() {
            let _: () = unsafe { msg_send![class, openSystemSettingsLoginItems] };
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod imp {
    use super::HelperState;

    pub fn state() -> HelperState {
        HelperState::Unavailable
    }

    pub fn register() -> Result<(), String> {
        Err("macOS only".into())
    }

    pub fn unregister() -> Result<(), String> {
        Err("macOS only".into())
    }

    pub fn open_settings() {}
}

pub fn status() -> HelperStatus {
    HelperStatus { state: imp::state(), error: None }
}

/// Turn the helper on or off. Turning it on opens System Settings when macOS
/// wants the user's approval.
pub fn set(on: bool) -> HelperStatus {
    let result = if on { imp::register() } else { imp::unregister() };
    let state = imp::state();
    if on && state == HelperState::NeedsApproval {
        imp::open_settings();
    }
    HelperStatus { state, error: result.err() }
}

pub fn open_settings() {
    imp::open_settings();
}

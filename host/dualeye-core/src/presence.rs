//! The person at the computer, for the board's pet (firmware 1.4): the local
//! time, for its day (lively by day, sleepy at night), and how long since
//! the last mouse or keyboard input, for "away" and "back". Sent as the
//! snapshot's `pet`.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Presence {
    /// Minutes since local midnight.
    pub min: u16,
    /// Seconds without input; absent where the OS won't tell (Linux), and
    /// the board then takes you as there.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idle: Option<u32>,
}

/// Now.
pub fn sample() -> Presence {
    use chrono::Timelike;
    let now = chrono::Local::now();
    Presence { min: (now.hour() * 60 + now.minute()) as u16, idle: platform::idle_seconds() }
}

#[cfg(target_os = "macos")]
mod platform {
    /// kCGEventSourceStateCombinedSessionState
    const COMBINED_SESSION: i32 = 0;
    /// kCGAnyInputEventType
    const ANY_INPUT: u32 = !0;

    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGEventSourceSecondsSinceLastEventType(state: i32, event_type: u32) -> f64;
    }

    pub(super) fn idle_seconds() -> Option<u32> {
        // SAFETY: a plain query with no pointers.
        let s = unsafe { CGEventSourceSecondsSinceLastEventType(COMBINED_SESSION, ANY_INPUT) };
        (s.is_finite() && s >= 0.0).then_some(s as u32)
    }
}

#[cfg(target_os = "windows")]
mod platform {
    #[repr(C)]
    struct LastInputInfo {
        size: u32,
        time: u32,
    }

    #[link(name = "user32")]
    unsafe extern "system" {
        fn GetLastInputInfo(info: *mut LastInputInfo) -> i32;
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetTickCount() -> u32;
    }

    pub(super) fn idle_seconds() -> Option<u32> {
        let mut info = LastInputInfo { size: size_of::<LastInputInfo>() as u32, time: 0 };
        // SAFETY: plain Win32 calls with a valid, sized out struct.
        unsafe {
            if GetLastInputInfo(&mut info) == 0 {
                return None;
            }
            Some(GetTickCount().wrapping_sub(info.time) / 1000)
        }
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
mod platform {
    pub(super) fn idle_seconds() -> Option<u32> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minutes_are_within_a_day() {
        assert!(sample().min < 24 * 60);
    }

    #[test]
    fn idle_is_left_out_when_unknown() {
        let json = serde_json::to_string(&Presence { min: 600, idle: None }).unwrap();
        assert_eq!(json, r#"{"min":600}"#);
    }
}

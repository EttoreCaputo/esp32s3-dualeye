//! Where the mouse pointer is, for the eyes face (firmware 1.3): the board's
//! eyes look at it. [`gaze`] puts it on the board's scale, -1..1 across all
//! the screens each way; the bridge sends it as `eyes/gaze` while it moves.
//!
//! macOS asks CoreGraphics, Windows `GetCursorPos`, Linux X11 (loaded at
//! run time; under Wayland only while the pointer is over an X window).
//! None of them needs a permission.

/// The pointer and the screens around it, in the OS's own units.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pointer {
    pub x: f64,
    pub y: f64,
    /// The box round every screen: left, top, width, height.
    pub bounds: (f64, f64, f64, f64),
}

impl Pointer {
    /// -1 (left, top) to 1 (right, bottom), to three decimals.
    pub fn gaze(&self) -> (f32, f32) {
        let (left, top, w, h) = self.bounds;
        let scale = |v: f64, start: f64, len: f64| {
            let t = if len > 0.0 { ((v - start) / len).clamp(0.0, 1.0) } else { 0.5 };
            ((t * 2.0 - 1.0) * 1000.0).round() as f32 / 1000.0
        };
        (scale(self.x, left, w), scale(self.y, top, h))
    }
}

/// The pointer now, or `None` where the OS won't tell.
pub fn pointer() -> Option<Pointer> {
    platform::pointer()
}

/// The pointer on the board's scale (see [`Pointer::gaze`]).
pub fn gaze() -> Option<(f32, f32)> {
    pointer().map(|p| p.gaze())
}

#[cfg(target_os = "macos")]
mod platform {
    use super::Pointer;
    use std::ffi::c_void;

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct CGPoint {
        x: f64,
        y: f64,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct CGSize {
        width: f64,
        height: f64,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct CGRect {
        origin: CGPoint,
        size: CGSize,
    }

    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGEventCreate(source: *const c_void) -> *mut c_void;
        fn CGEventGetLocation(event: *mut c_void) -> CGPoint;
        fn CGGetActiveDisplayList(max: u32, displays: *mut u32, count: *mut u32) -> i32;
        fn CGDisplayBounds(display: u32) -> CGRect;
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFRelease(cf: *const c_void);
    }

    pub(super) fn pointer() -> Option<Pointer> {
        // SAFETY: a NULL source makes an event that only carries the pointer;
        // it's released right after. The display list fills at most 16 ids.
        unsafe {
            let event = CGEventCreate(std::ptr::null());
            if event.is_null() {
                return None;
            }
            let at = CGEventGetLocation(event);
            CFRelease(event);
            let mut ids = [0u32; 16];
            let mut count = 0u32;
            if CGGetActiveDisplayList(ids.len() as u32, ids.as_mut_ptr(), &mut count) != 0 || count == 0 {
                return None;
            }
            let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
            for &id in &ids[..count as usize] {
                let r = CGDisplayBounds(id);
                x0 = x0.min(r.origin.x);
                y0 = y0.min(r.origin.y);
                x1 = x1.max(r.origin.x + r.size.width);
                y1 = y1.max(r.origin.y + r.size.height);
            }
            Some(Pointer { x: at.x, y: at.y, bounds: (x0, y0, x1 - x0, y1 - y0) })
        }
    }
}

#[cfg(target_os = "windows")]
mod platform {
    use super::Pointer;

    #[repr(C)]
    struct Point {
        x: i32,
        y: i32,
    }

    const SM_XVIRTUALSCREEN: i32 = 76;
    const SM_YVIRTUALSCREEN: i32 = 77;
    const SM_CXVIRTUALSCREEN: i32 = 78;
    const SM_CYVIRTUALSCREEN: i32 = 79;

    #[link(name = "user32")]
    unsafe extern "system" {
        fn GetCursorPos(point: *mut Point) -> i32;
        fn GetSystemMetrics(index: i32) -> i32;
    }

    pub(super) fn pointer() -> Option<Pointer> {
        let mut p = Point { x: 0, y: 0 };
        // SAFETY: plain Win32 calls with a valid out pointer.
        unsafe {
            if GetCursorPos(&mut p) == 0 {
                return None;
            }
            let bounds = (
                f64::from(GetSystemMetrics(SM_XVIRTUALSCREEN)),
                f64::from(GetSystemMetrics(SM_YVIRTUALSCREEN)),
                f64::from(GetSystemMetrics(SM_CXVIRTUALSCREEN)),
                f64::from(GetSystemMetrics(SM_CYVIRTUALSCREEN)),
            );
            Some(Pointer { x: f64::from(p.x), y: f64::from(p.y), bounds })
        }
    }
}

#[cfg(target_os = "linux")]
mod platform {
    use super::Pointer;
    use std::sync::{Mutex, OnceLock};
    use x11_dl::xlib::{Display, Xlib};

    /// libX11 and a connection to the display, opened once; `None` without X.
    struct X {
        xlib: Xlib,
        display: *mut Display,
    }

    // SAFETY: the display is only used under the mutex below.
    unsafe impl Send for X {}

    fn x() -> &'static Mutex<Option<X>> {
        static X11: OnceLock<Mutex<Option<X>>> = OnceLock::new();
        X11.get_or_init(|| {
            let opened = Xlib::open().ok().and_then(|xlib| {
                // SAFETY: NULL opens $DISPLAY; a NULL result means no X server.
                let display = unsafe { (xlib.XOpenDisplay)(std::ptr::null()) };
                (!display.is_null()).then_some(X { xlib, display })
            });
            Mutex::new(opened)
        })
    }

    pub(super) fn pointer() -> Option<Pointer> {
        let guard = x().lock().ok()?;
        let x = guard.as_ref()?;
        let (mut root, mut child) = (0, 0);
        let (mut rx, mut ry, mut wx, mut wy, mut mask) = (0, 0, 0, 0, 0);
        // SAFETY: the display is open and only used under the lock.
        unsafe {
            let screen = (x.xlib.XDefaultScreen)(x.display);
            let window = (x.xlib.XRootWindow)(x.display, screen);
            if (x.xlib.XQueryPointer)(x.display, window, &mut root, &mut child, &mut rx, &mut ry, &mut wx, &mut wy, &mut mask) == 0 {
                return None;
            }
            let w = (x.xlib.XDisplayWidth)(x.display, screen);
            let h = (x.xlib.XDisplayHeight)(x.display, screen);
            Some(Pointer { x: f64::from(rx), y: f64::from(ry), bounds: (0.0, 0.0, f64::from(w), f64::from(h)) })
        }
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
mod platform {
    pub(super) fn pointer() -> Option<super::Pointer> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gaze_spans_all_screens() {
        // A second screen to the left of the main one.
        let p = |x, y| Pointer { x, y, bounds: (-1920.0, 0.0, 3840.0, 1080.0) };
        assert_eq!(p(0.0, 540.0).gaze(), (0.0, 0.0));
        assert_eq!(p(-1920.0, 0.0).gaze(), (-1.0, -1.0));
        assert_eq!(p(1920.0, 1080.0).gaze(), (1.0, 1.0));
        assert_eq!(p(5000.0, -10.0).gaze(), (1.0, -1.0));
        assert_eq!(p(960.0, 270.0).gaze(), (0.5, -0.5));
    }
}

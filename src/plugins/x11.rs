//! a bare x11 toplevel window to embed clap editors into.
//!
//! clap on linux only standardises x11 embedding, and egui's wayland
//! window can't be an x11 parent, so editors get their own window through
//! xwayland. only the handful of xlib calls needed are declared here.

use std::ffi::{CString, c_char, c_int, c_long, c_uint, c_ulong, c_void};

type Display = c_void;
type Window = c_ulong;
type Atom = c_ulong;

/// xevent is a union padded to 24 longs
#[repr(C)]
struct XEvent([c_long; 24]);

#[repr(C)]
struct XClassHint {
    res_name: *const c_char,
    res_class: *const c_char,
}

#[link(name = "X11")]
unsafe extern "C" {
    fn XOpenDisplay(name: *const c_char) -> *mut Display;
    fn XCloseDisplay(display: *mut Display) -> c_int;
    fn XDefaultScreen(display: *mut Display) -> c_int;
    fn XRootWindow(display: *mut Display, screen: c_int) -> Window;
    fn XBlackPixel(display: *mut Display, screen: c_int) -> c_ulong;
    fn XCreateSimpleWindow(
        display: *mut Display,
        parent: Window,
        x: c_int,
        y: c_int,
        width: c_uint,
        height: c_uint,
        border_width: c_uint,
        border: c_ulong,
        background: c_ulong,
    ) -> Window;
    fn XStoreName(display: *mut Display, w: Window, name: *const c_char) -> c_int;
    fn XSetClassHint(display: *mut Display, w: Window, hint: *mut XClassHint) -> c_int;
    fn XSelectInput(display: *mut Display, w: Window, mask: c_long) -> c_int;
    fn XInternAtom(display: *mut Display, name: *const c_char, only_if_exists: c_int) -> Atom;
    fn XSetWMProtocols(
        display: *mut Display,
        w: Window,
        protocols: *mut Atom,
        count: c_int,
    ) -> c_int;
    fn XMapRaised(display: *mut Display, w: Window) -> c_int;
    fn XResizeWindow(display: *mut Display, w: Window, width: c_uint, height: c_uint) -> c_int;
    fn XDestroyWindow(display: *mut Display, w: Window) -> c_int;
    fn XPending(display: *mut Display) -> c_int;
    fn XNextEvent(display: *mut Display, event: *mut XEvent) -> c_int;
    fn XFlush(display: *mut Display) -> c_int;
}

const STRUCTURE_NOTIFY_MASK: c_long = 1 << 17;
const CONFIGURE_NOTIFY: c_int = 22;
const CLIENT_MESSAGE: c_int = 33;

pub enum WindowEvent {
    CloseRequested,
    Resized(u32, u32),
}

pub struct HostWindow {
    display: *mut Display,
    window: Window,
    wm_delete: Atom,
    size: (u32, u32),
}

impl HostWindow {
    /// `resizable` is left to the window manager on x11
    pub fn new(title: &str, width: u32, height: u32, _resizable: bool) -> anyhow::Result<Self> {
        // safety: plain xlib calls on a connection we own
        unsafe {
            let display = XOpenDisplay(std::ptr::null());
            if display.is_null() {
                anyhow::bail!("cannot open x11 display (is xwayland running?)");
            }
            let screen = XDefaultScreen(display);
            let black = XBlackPixel(display, screen);
            let window = XCreateSimpleWindow(
                display,
                XRootWindow(display, screen),
                0,
                0,
                width.max(1),
                height.max(1),
                0,
                black,
                black,
            );
            let title = CString::new(title.replace('\0', "")).unwrap_or_default();
            XStoreName(display, window, title.as_ptr());
            // lets users write window rules, e.g. hyprland `windowrule = float, class:fsa-plugin`
            let mut class = XClassHint {
                res_name: c"fsa-plugin".as_ptr(),
                res_class: c"fsa-plugin".as_ptr(),
            };
            XSetClassHint(display, window, &mut class);
            XSelectInput(display, window, STRUCTURE_NOTIFY_MASK);
            let mut wm_delete = XInternAtom(display, c"WM_DELETE_WINDOW".as_ptr(), 0);
            XSetWMProtocols(display, window, &mut wm_delete, 1);
            XMapRaised(display, window);
            XFlush(display);
            Ok(Self {
                display,
                window,
                wm_delete,
                size: (width, height),
            })
        }
    }

    pub fn clap_window(&self) -> clack_extensions::gui::Window<'static, 'static> {
        clack_extensions::gui::Window::from_x11_handle(self.window)
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        self.size = (width, height);
        // safety: window and display are valid until drop
        unsafe {
            XResizeWindow(self.display, self.window, width.max(1), height.max(1));
            XFlush(self.display);
        }
    }

    /// drains pending x events without blocking
    pub fn poll(&mut self) -> Vec<WindowEvent> {
        let mut out = Vec::new();
        // safety: xevent is large enough for every event type
        unsafe {
            while XPending(self.display) > 0 {
                let mut ev = XEvent([0; 24]);
                XNextEvent(self.display, &mut ev);
                let kind = ev.0[0] as c_int;
                if kind == CLIENT_MESSAGE && ev.0[7] as Atom == self.wm_delete {
                    // xclientmessageevent: data.l[0] sits at byte 56
                    out.push(WindowEvent::CloseRequested);
                } else if kind == CONFIGURE_NOTIFY {
                    // xconfigureevent: width and height are ints at bytes 56 and 60
                    let packed = ev.0[7] as u64;
                    let (w, h) = (packed as u32, (packed >> 32) as u32);
                    if (w, h) != self.size && w > 0 && h > 0 {
                        self.size = (w, h);
                        out.push(WindowEvent::Resized(w, h));
                    }
                }
            }
        }
        out
    }
}

impl Drop for HostWindow {
    fn drop(&mut self) {
        // safety: we own both
        unsafe {
            XDestroyWindow(self.display, self.window);
            XCloseDisplay(self.display);
        }
    }
}

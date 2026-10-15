//! a plain cocoa window to embed clap editors into on macos.
//!
//! events are polled from the gui loop (visibility and content size)
//! instead of installing a window delegate.

use std::ffi::c_void;

use anyhow::Context as _;
use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2_app_kit::{NSBackingStoreType, NSWindow, NSWindowStyleMask};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};

pub enum WindowEvent {
    CloseRequested,
    Resized(u32, u32),
}

pub struct HostWindow {
    window: Retained<NSWindow>,
    size: (u32, u32),
}

impl HostWindow {
    pub fn new(title: &str, width: u32, height: u32, resizable: bool) -> anyhow::Result<Self> {
        let mtm =
            MainThreadMarker::new().context("editor windows must be created on the main thread")?;
        let mut style = NSWindowStyleMask::Titled
            | NSWindowStyleMask::Closable
            | NSWindowStyleMask::Miniaturizable;
        if resizable {
            style |= NSWindowStyleMask::Resizable;
        }
        let rect = NSRect::new(
            NSPoint::new(0.0, 0.0),
            NSSize::new(width as f64, height as f64),
        );
        // safety: standard initialiser on the main thread
        let window = unsafe {
            NSWindow::initWithContentRect_styleMask_backing_defer(
                mtm.alloc(),
                rect,
                style,
                NSBackingStoreType::Buffered,
                false,
            )
        };
        // we own the window; closing it must not free it behind our back
        unsafe { window.setReleasedWhenClosed(false) };
        window.setTitle(&NSString::from_str(title));
        window.center();
        window.makeKeyAndOrderFront(None);
        Ok(Self {
            window,
            size: (width, height),
        })
    }

    /// the content view as a `clap_window` (cocoa nsview)
    pub fn clap_window(&self) -> clack_extensions::gui::Window<'static, 'static> {
        let view = self.window.contentView().map_or(std::ptr::null_mut(), |v| {
            Retained::as_ptr(&v) as *mut c_void
        });
        // safety: the view belongs to our window, which outlives the plugin gui
        unsafe { clack_extensions::gui::Window::from_cocoa_nsview(view) }
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        self.size = (width, height);
        self.window
            .setContentSize(NSSize::new(width as f64, height as f64));
    }

    pub fn poll(&mut self) -> Vec<WindowEvent> {
        // releasedWhenClosed is off, so a closed window just stops being visible
        if !self.window.isVisible() && !self.window.isMiniaturized() {
            return vec![WindowEvent::CloseRequested];
        }
        let Some(view) = self.window.contentView() else {
            return Vec::new();
        };
        let frame = view.frame();
        let size = (
            frame.size.width.round() as u32,
            frame.size.height.round() as u32,
        );
        if size != self.size && size.0 > 0 && size.1 > 0 {
            self.size = size;
            return vec![WindowEvent::Resized(size.0, size.1)];
        }
        Vec::new()
    }
}

impl Drop for HostWindow {
    fn drop(&mut self) {
        self.window.close();
    }
}

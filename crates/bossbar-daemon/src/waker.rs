//! Cross-thread wake-up plumbing for the daemon's event loop.

use std::sync::{Arc, OnceLock};

#[cfg(windows)]
use crate::platform;

/// Lets the IPC and tray threads poke the eframe event loop.
///
/// The daemon parks its event loop in `ControlFlow::Wait` while nothing is
/// animating; `wake` clears that parking by requesting a repaint. On Windows a
/// repaint request alone cannot wake a parked pill: the system never delivers
/// paint messages to hidden windows, so the window is shown again (without
/// activation) first. The UI then decides on that frame whether the pill stays
/// or the window is hidden again.
#[derive(Clone, Default)]
pub struct Waker {
    inner: Arc<WakerInner>,
}

#[derive(Default)]
struct WakerInner {
    ctx: OnceLock<egui::Context>,
    /// Native window handle, registered on the first frame. Windows only:
    /// elsewhere a repaint request reaches hidden windows on its own.
    #[cfg(windows)]
    hwnd: OnceLock<isize>,
}

impl Waker {
    /// Called once by the app after the eframe context exists.
    pub fn set_context(&self, ctx: egui::Context) {
        let _ = self.inner.ctx.set(ctx);
    }

    /// Called once by the app as soon as the native window exists; lets
    /// [`Self::wake`] make a hidden window paintable again.
    #[cfg(windows)]
    pub fn set_hwnd(&self, hwnd: isize) {
        let _ = self.inner.hwnd.set(hwnd);
    }

    pub fn wake(&self) {
        if let Some(ctx) = self.inner.ctx.get() {
            ctx.request_repaint();
        }
        #[cfg(windows)]
        if let Some(hwnd) = self.inner.hwnd.get() {
            platform::show_if_hidden(*hwnd);
        }
    }
}

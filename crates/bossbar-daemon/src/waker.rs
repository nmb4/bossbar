//! Cross-thread wake-up plumbing for the daemon's event loop.

use std::sync::{Arc, OnceLock};

/// Lets the IPC and tray threads poke the eframe event loop.
///
/// The daemon parks its event loop in `ControlFlow::Wait` while nothing is
/// animating; `wake` clears that parking by requesting a repaint, which wakes
/// winit and delivers a frame even when the window is currently hidden.
#[derive(Clone, Default)]
pub struct Waker {
    inner: Arc<WakerInner>,
}

#[derive(Default)]
struct WakerInner {
    ctx: OnceLock<egui::Context>,
}

impl Waker {
    /// Called once by the app after the eframe context exists.
    pub fn set_context(&self, ctx: egui::Context) {
        let _ = self.inner.ctx.set(ctx);
    }

    pub fn wake(&self) {
        if let Some(ctx) = self.inner.ctx.get() {
            ctx.request_repaint();
        }
    }
}

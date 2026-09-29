//! Event-loop bootstrap.
//!
//! eframe's `run_native` cannot express two things the daemon needs:
//! - macOS accessory activation policy, so the background process has no Dock
//!   icon and never steals focus;
//! - a default `ControlFlow::Wait`, so the process stays at 0% CPU while the
//!   pill is hidden and idle.
//!
//! We therefore build the winit event loop ourselves and hand it to eframe.
//! eframe 0.33 skips drawing invisible windows, but on Windows a repaint that
//! expires after the window hides must also be parked explicitly so its
//! temporary `ControlFlow::Poll` does not become permanent.

use eframe::{AppCreator, NativeOptions, UserEvent};
use winit::event_loop::{ControlFlow, EventLoop};
#[cfg(windows)]
use winit::{
    application::ApplicationHandler,
    event::{DeviceEvent, DeviceId, StartCause, WindowEvent},
    event_loop::ActiveEventLoop,
    window::WindowId,
};

/// Keeps a repaint aimed at an invisible Windows window from turning into a
/// permanent busy loop.
///
/// eframe temporarily selects `ControlFlow::Poll` when a repaint deadline is
/// due and relies on `RedrawRequested` to put the loop back to sleep. Windows
/// does not emit that event for hidden windows. If no window event arrived in
/// the batch, `about_to_wait` is the last safe point to restore `Wait`; a
/// queued redraw for a visible window still wakes the loop normally.
#[cfg(windows)]
struct ParkMissedRedraw<A> {
    inner: A,
}

#[cfg(windows)]
impl<A: ApplicationHandler<UserEvent>> ApplicationHandler<UserEvent> for ParkMissedRedraw<A> {
    fn new_events(&mut self, event_loop: &ActiveEventLoop, cause: StartCause) {
        self.inner.new_events(event_loop, cause);
    }

    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        self.inner.resumed(event_loop);
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: UserEvent) {
        self.inner.user_event(event_loop, event);
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        self.inner.window_event(event_loop, window_id, event);
    }

    fn device_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        device_id: DeviceId,
        event: DeviceEvent,
    ) {
        self.inner.device_event(event_loop, device_id, event);
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        self.inner.about_to_wait(event_loop);
        if matches!(event_loop.control_flow(), ControlFlow::Poll) {
            event_loop.set_control_flow(ControlFlow::Wait);
        }
    }

    fn suspended(&mut self, event_loop: &ActiveEventLoop) {
        self.inner.suspended(event_loop);
    }

    fn exiting(&mut self, event_loop: &ActiveEventLoop) {
        self.inner.exiting(event_loop);
    }

    fn memory_warning(&mut self, event_loop: &ActiveEventLoop) {
        self.inner.memory_warning(event_loop);
    }
}

pub fn run(app_name: &str, options: NativeOptions, app_creator: AppCreator<'_>) -> eframe::Result {
    #[cfg(target_os = "macos")]
    let event_loop = {
        use winit::platform::macos::{ActivationPolicy, EventLoopBuilderExtMacOS as _};
        let mut builder = EventLoop::<UserEvent>::with_user_event();
        // Accessory: menu-bar (tray) app without a Dock icon.
        builder.with_activation_policy(ActivationPolicy::Accessory);
        builder.with_default_menu(false);
        // Do not force the daemon to the front at launch; it is a background
        // process and must never interrupt whatever the user is doing.
        builder.with_activate_ignoring_other_apps(false);
        builder.build()
    };
    #[cfg(not(target_os = "macos"))]
    let event_loop = EventLoop::<UserEvent>::with_user_event().build();

    let event_loop = event_loop.map_err(eframe::Error::WinitEventLoop)?;
    event_loop.set_control_flow(ControlFlow::Wait);
    let app = eframe::create_native(app_name, options, app_creator, &event_loop);
    #[cfg(windows)]
    let mut app = ParkMissedRedraw { inner: app };
    #[cfg(not(windows))]
    let mut app = app;
    event_loop
        .run_app(&mut app)
        .map_err(eframe::Error::WinitEventLoop)
}

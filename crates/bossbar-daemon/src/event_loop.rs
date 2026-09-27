//! Event-loop bootstrap.
//!
//! eframe's `run_native` cannot express two things the daemon needs:
//! - macOS accessory activation policy, so the background process has no Dock
//!   icon and never steals focus;
//! - a default `ControlFlow::Wait`, so the process stays at 0% CPU while the
//!   pill is hidden and idle.
//!
//! We therefore build the winit event loop ourselves and hand it to eframe.
//! eframe 0.33 already skips redraw work for invisible windows, so no custom
//! redraw suppression is needed.

use eframe::{AppCreator, NativeOptions, UserEvent};
use winit::event_loop::{ControlFlow, EventLoop};

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
    let mut app = eframe::create_native(app_name, options, app_creator, &event_loop);
    event_loop
        .run_app(&mut app)
        .map_err(eframe::Error::WinitEventLoop)
}

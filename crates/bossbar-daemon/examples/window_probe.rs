//! Debug helper: prints the geometry of the bossbar pill window and the
//! monitor layout, so placement can be verified without screenshots.
//!
//! Run with the daemon up: `cargo run -p bossbar-daemon --example window_probe`

fn main() -> anyhow::Result<()> {
    println!("monitors:");
    for monitor in xcap::Monitor::all()? {
        println!(
            "  {} @ ({}, {}) {}x{} scale={} primary={}",
            monitor.name()?,
            monitor.x()?,
            monitor.y()?,
            monitor.width()?,
            monitor.height()?,
            monitor.scale_factor()?,
            monitor.is_primary()?
        );
    }

    let mut found = false;
    for window in xcap::Window::all()? {
        let title = window.title().unwrap_or_default();
        if !title.contains("bossbar") {
            continue;
        }
        found = true;
        println!(
            "window '{}': pos=({}, {}) size={}x{} monitor={} app={} z={} focused={}",
            title,
            window.x()?,
            window.y()?,
            window.width()?,
            window.height()?,
            window.current_monitor()?.name()?,
            window.app_name().unwrap_or_default(),
            window.z().unwrap_or(-1),
            window.is_focused().unwrap_or(false),
        );
    }
    if !found {
        println!("no visible bossbar window (window is hidden while no bars exist)");
    }
    Ok(())
}

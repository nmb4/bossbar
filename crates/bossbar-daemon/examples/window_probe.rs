//! Debug helper: prints the geometry of the bossbar pill window and the
//! monitor layout, so placement can be verified without screenshots.
//!
//! Run with the daemon up: `cargo run -p bossbar-daemon --example window_probe`
//! Pass a PNG path after `--` to also capture the visible pill window.
//! Add `--desktop` after the path to include the desktop beneath transparent pixels.

fn main() -> anyhow::Result<()> {
    let screenshot = std::env::args().nth(1);
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

    #[cfg(windows)]
    {
        use windows::Win32::Foundation::POINT;
        use windows::Win32::Graphics::Gdi::{
            GetMonitorInfoW, MonitorFromPoint, MONITORINFO, MONITOR_DEFAULTTONEAREST,
        };
        use windows::Win32::UI::WindowsAndMessaging::{
            GetCursorPos, GetSystemMetrics, SM_CXSCREEN, SM_CXVIRTUALSCREEN,
        };

        println!("win32:");
        let mut cursor = POINT::default();
        unsafe { GetCursorPos(&mut cursor)? };
        println!("  cursor: ({}, {})", cursor.x, cursor.y);
        println!(
            "  SM_CXSCREEN: {}  SM_CXVIRTUALSCREEN: {}",
            unsafe { GetSystemMetrics(SM_CXSCREEN) },
            unsafe { GetSystemMetrics(SM_CXVIRTUALSCREEN) }
        );
        let hmonitor = unsafe { MonitorFromPoint(cursor, MONITOR_DEFAULTTONEAREST) };
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if unsafe { GetMonitorInfoW(hmonitor, &mut info) }.as_bool() {
            println!(
                "  MonitorFromPoint: mon={:?} work={:?}",
                info.rcMonitor, info.rcWork
            );
        }
        match xcap::Monitor::from_point(cursor.x, cursor.y) {
            Ok(monitor) => println!(
                "  xcap::from_point(cursor): ({}, {}) {}x{} scale={}",
                monitor.x()?,
                monitor.y()?,
                monitor.width()?,
                monitor.height()?,
                monitor.scale_factor()?
            ),
            Err(error) => println!("  xcap::from_point(cursor) failed: {error}"),
        }
    }

    let mut found = false;
    for window in xcap::Window::all()? {
        let title = window.title().unwrap_or_default();
        if !title.contains("bossbar") {
            continue;
        }
        found = true;
        if title == "bossbar" {
            if let Some(path) = &screenshot {
                if std::env::args().any(|arg| arg == "--desktop") {
                    let monitor = window.current_monitor()?;
                    let scale = if cfg!(windows) {
                        monitor.scale_factor()?
                    } else {
                        1.0
                    };
                    let desktop = monitor.capture_image()?;
                    let x = (window.x()? as f32 * scale).round() as i32 - monitor.x()?;
                    let y = (window.y()? as f32 * scale).round() as i32 - monitor.y()?;
                    image::imageops::crop_imm(
                        &desktop,
                        x.max(0) as u32,
                        y.max(0) as u32,
                        (window.width()? as f32 * scale).round() as u32,
                        (window.height()? as f32 * scale).round() as u32,
                    )
                    .to_image()
                    .save(path)?;
                } else {
                    window.capture_image()?.save(path)?;
                }
                println!("saved pill screenshot: {path}");
            }
        }
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

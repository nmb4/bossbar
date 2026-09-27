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

    #[cfg(windows)]
    {
        use windows::Win32::Foundation::POINT;
        use windows::Win32::Graphics::Gdi::{
            GetMonitorInfoW, MonitorFromPoint, MONITOR_DEFAULTTONEAREST, MONITORINFO,
        };
        use windows::Win32::UI::WindowsAndMessaging::{
            GetCursorPos, GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_CXSCREEN,
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

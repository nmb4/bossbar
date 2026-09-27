//! Monitor lookup and native window placement.
//!
//! Units are platform-specific and matter:
//! - macOS: [`MonitorInfo`] coordinates come from `CGDisplayBounds`, which is
//!   already in logical points, matching egui and AppKit.
//! - Windows/Linux: monitor coordinates are physical pixels; egui window
//!   positions are logical points, so we divide by the *window's current*
//!   scale factor. winit converts back with that same factor, which keeps
//!   moves between mixed-DPI monitors exact.

use bossbar_proto::Anchor;
use egui::{Pos2, Vec2, ViewportCommand};

/// Distance from the very top of the monitor to the pill, in screen units.
const MACOS_MENU_BAR_POINTS: f32 = 38.0;
const GENERIC_TOP_MARGIN: f32 = 12.0;
/// Distance from the right monitor edge when anchored top-right.
const RIGHT_INSET: f32 = 12.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MonitorInfo {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub scale: f32,
    /// `true` when coordinates are logical points (macOS).
    pub points: bool,
}

impl MonitorInfo {
    fn from_xcap(monitor: &xcap::Monitor) -> Option<Self> {
        Some(Self {
            x: monitor.x().ok()?,
            y: monitor.y().ok()?,
            width: monitor.width().ok()? as i32,
            height: monitor.height().ok()? as i32,
            scale: monitor.scale_factor().ok()?.max(0.5),
            points: cfg!(target_os = "macos"),
        })
    }

    fn unit_scale(&self) -> f32 {
        if self.points {
            1.0
        } else {
            self.scale
        }
    }
}

/// Monitor under the mouse cursor, falling back to the primary monitor.
pub fn active_monitor() -> Option<MonitorInfo> {
    if let Some((x, y)) = cursor_position() {
        if let Ok(monitor) = xcap::Monitor::from_point(x, y) {
            if let Some(info) = MonitorInfo::from_xcap(&monitor) {
                return Some(info);
            }
        }
    }
    primary_monitor()
}

pub fn primary_monitor() -> Option<MonitorInfo> {
    let monitors = xcap::Monitor::all().ok()?;
    monitors
        .iter()
        .find(|monitor| monitor.is_primary().unwrap_or(false))
        .or_else(|| monitors.first())
        .and_then(MonitorInfo::from_xcap)
}

/// Moves the window to `anchor` on `monitor` for a pill of `pill_size` points.
pub fn place_window(ctx: &egui::Context, monitor: MonitorInfo, anchor: Anchor, pill_size: Vec2) {
    let unit = monitor.unit_scale();
    let width_units = pill_size.x * unit;
    let y_units = monitor.y as f32 + top_margin_units(&monitor);
    let x_units = match anchor {
        Anchor::TopCenter => monitor.x as f32 + (monitor.width as f32 - width_units) / 2.0,
        Anchor::TopRight => {
            monitor.x as f32 + monitor.width as f32 - width_units - RIGHT_INSET * unit
        }
    };

    // winit logical positions: points on macOS, points = physical / window
    // scale factor elsewhere.
    let logical = if monitor.points {
        Pos2::new(x_units, y_units)
    } else {
        let window_scale = ctx.pixels_per_point().max(0.5);
        Pos2::new(x_units / window_scale, y_units / window_scale)
    };
    ctx.send_viewport_cmd(ViewportCommand::OuterPosition(logical));
}

fn top_margin_units(monitor: &MonitorInfo) -> f32 {
    let base = if cfg!(target_os = "macos") {
        MACOS_MENU_BAR_POINTS
    } else {
        GENERIC_TOP_MARGIN
    };
    base * monitor.unit_scale()
}

/// Cursor position in the same unit space as [`MonitorInfo`] (points on
/// macOS, physical pixels elsewhere).
#[cfg(target_os = "macos")]
fn cursor_position() -> Option<(i32, i32)> {
    use core_graphics::event::CGEvent;
    use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};

    let source = CGEventSource::new(CGEventSourceStateID::HIDSystemState).ok()?;
    let event = CGEvent::new(source).ok()?;
    let point = event.location();
    Some((point.x as i32, point.y as i32))
}

#[cfg(windows)]
fn cursor_position() -> Option<(i32, i32)> {
    use windows::Win32::Foundation::POINT;
    use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;

    let mut point = POINT::default();
    unsafe { GetCursorPos(&mut point).ok()? };
    Some((point.x, point.y))
}

#[cfg(all(not(target_os = "macos"), not(windows)))]
fn cursor_position() -> Option<(i32, i32)> {
    // Best effort on Linux: fall back to the primary monitor.
    None
}

/// Native window handle, used for the shaped region on Windows.
#[cfg(windows)]
pub fn window_hwnd(frame: &eframe::Frame) -> Option<isize> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    match frame.window_handle().ok()?.as_raw() {
        RawWindowHandle::Win32(handle) => Some(handle.hwnd.get()),
        _ => None,
    }
}

/// Marks the macOS window as non-activating.
///
/// winit shows the pill with `makeKeyAndOrderFront:`, which would otherwise
/// pull the daemon (and with it the user's focus) to the front every time a
/// bar appears. `_setPreventsActivation:` is the long-standing AppKit
/// workaround used by overlay utilities; it is applied only when the runtime
/// says the selector exists.
#[cfg(target_os = "macos")]
pub fn macos_prevent_activation(frame: &eframe::Frame) -> bool {
    use objc2::runtime::{AnyObject, Bool, Sel};
    use objc2::{msg_send, sel};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    let Ok(handle) = frame.window_handle() else {
        return false;
    };
    let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
        return false;
    };

    unsafe {
        let view = handle.ns_view.as_ptr() as *mut AnyObject;
        if view.is_null() {
            return false;
        }
        let view = &*view;
        let window: *mut AnyObject = msg_send![view, window];
        if window.is_null() {
            return false;
        }
        let window = &*window;
        let selector: Sel = sel!(_setPreventsActivation:);
        let responds: Bool = msg_send![window, respondsToSelector: selector];
        if !responds.as_bool() {
            return false;
        }
        let (): () = msg_send![window, _setPreventsActivation: Bool::YES];
    }
    true
}

/// Trims the Windows window to a rounded rectangle so the transparent surface
/// does not need per-pixel alpha.
#[cfg(windows)]
pub fn apply_window_region(
    hwnd: isize,
    size_points: Vec2,
    pixels_per_point: f32,
    radius_points: f32,
    applied: &mut Option<(i32, i32)>,
) {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::Graphics::Gdi::{CreateRoundRectRgn, DeleteObject, SetWindowRgn};

    let width = (size_points.x * pixels_per_point).round().max(1.0) as i32;
    let height = (size_points.y * pixels_per_point).round().max(1.0) as i32;
    if *applied == Some((width, height)) {
        return;
    }
    let diameter = (radius_points * 2.0 * pixels_per_point).round().max(2.0) as i32;
    unsafe {
        let region = CreateRoundRectRgn(0, 0, width + 1, height + 1, diameter, diameter);
        if region.is_invalid() {
            return;
        }
        let window = HWND(hwnd as *mut std::ffi::c_void);
        if SetWindowRgn(window, Some(region), true) != 0 {
            // SetWindowRgn owns the region after a successful call.
            *applied = Some((width, height));
        } else {
            let _ = DeleteObject(region.into());
        }
    }
}

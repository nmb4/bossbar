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

/// macOS keeps the pill below the menu bar even when flush.
const MACOS_MENU_BAR_POINTS: f32 = 38.0;
/// Added around the pill when the padding option is enabled.
const EXTRA_PADDING_POINTS: f32 = 12.0;

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
///
/// With `padding` disabled the pill sits flush against the usable screen
/// bounds (the very top edge on Windows/Linux, just below the menu bar on
/// macOS); enabling it adds [`EXTRA_PADDING_POINTS`] of breathing room.
///
/// Returns the logical position that was requested. macOS may refuse to put a
/// window all the way against the top of the screen; callers compare the
/// requested origin against the window's actual one to compensate.
pub fn place_window(
    ctx: &egui::Context,
    monitor: MonitorInfo,
    anchor: Anchor,
    pill_size: Vec2,
    margin: f32,
    padding: bool,
) -> Pos2 {
    let (x_units, y_units) = window_origin_units(&monitor, anchor, pill_size, margin, padding);

    // winit logical positions: points on macOS, points = physical / window
    // scale factor elsewhere.
    let logical = if monitor.points {
        Pos2::new(x_units, y_units)
    } else {
        let window_scale = ctx.pixels_per_point().max(0.5);
        Pos2::new(x_units / window_scale, y_units / window_scale)
    };
    ctx.send_viewport_cmd(ViewportCommand::OuterPosition(logical));
    logical
}

/// Window origin in monitor units for the given pill placement.
///
/// Anchoring is computed on the *pill*, not the window: the transparent
/// shadow margin extends past the screen edge instead of pushing the visible
/// surface inward. With `padding` off the pill touches the screen bound.
fn window_origin_units(
    monitor: &MonitorInfo,
    anchor: Anchor,
    pill_size: Vec2,
    margin: f32,
    padding: bool,
) -> (f32, f32) {
    let unit = monitor.unit_scale();
    let margin_units = margin * unit;
    let pill_left_units = match anchor {
        Anchor::TopCenter => monitor.x as f32 + (monitor.width as f32 - pill_size.x * unit) / 2.0,
        Anchor::TopRight => {
            monitor.x as f32 + monitor.width as f32
                - pill_size.x * unit
                - side_inset_units(monitor, padding)
        }
    };
    let pill_top_units = monitor.y as f32 + top_margin_units(monitor, padding);
    (
        pill_left_units - margin_units,
        pill_top_units - margin_units,
    )
}

fn top_margin_units(monitor: &MonitorInfo, padding: bool) -> f32 {
    let base = if cfg!(target_os = "macos") {
        MACOS_MENU_BAR_POINTS
    } else {
        0.0
    };
    let extra = if padding { EXTRA_PADDING_POINTS } else { 0.0 };
    (base + extra) * monitor.unit_scale()
}

fn side_inset_units(monitor: &MonitorInfo, padding: bool) -> f32 {
    if padding {
        EXTRA_PADDING_POINTS * monitor.unit_scale()
    } else {
        0.0
    }
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

/// Shows a hidden window without activating it.
///
/// Windows only delivers paint messages to visible windows, so a repaint
/// request cannot wake the daemon once the pill has parked the window.
/// Showing it is what produces the `WM_PAINT` that runs a frame; the app
/// decides on that frame whether the pill stays or the window is hidden
/// again. Returns `true` when the window was hidden and is now shown.
///
/// Raw visibility changes stay in sync with winit's cached window flags:
/// every show is either followed by a hide on the same frame or by a
/// `ViewportCommand::Visible(true)`, so winit only ever diffs against the
/// real state. Keep it that way.
#[cfg(windows)]
pub fn show_if_hidden(hwnd: isize) -> bool {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{
        IsWindowVisible, ShowWindow, SW_SHOWNOACTIVATE,
    };

    let window = HWND(hwnd as *mut std::ffi::c_void);
    unsafe {
        if IsWindowVisible(window).as_bool() {
            return false;
        }
        tracing::trace!("showing a hidden window so it can be painted");
        let _ = ShowWindow(window, SW_SHOWNOACTIVATE);
        true
    }
}

/// Hides a shown window, undoing [`show_if_hidden`] when a wake found nothing
/// to display. Returns `true` when the window was shown and is now hidden.
#[cfg(windows)]
pub fn hide_if_visible(hwnd: isize) -> bool {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{IsWindowVisible, ShowWindow, SW_HIDE};

    let window = HWND(hwnd as *mut std::ffi::c_void);
    unsafe {
        if !IsWindowVisible(window).as_bool() {
            return false;
        }
        tracing::trace!("hiding the window again; nothing to show");
        let _ = ShowWindow(window, SW_HIDE);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PILL: Vec2 = Vec2::new(340.0, 50.0);
    const MARGIN: f32 = 10.0;

    fn monitor(points: bool, scale: f32) -> MonitorInfo {
        MonitorInfo {
            x: 0,
            y: 0,
            width: 1512,
            height: 982,
            scale,
            points,
        }
    }

    #[test]
    fn flush_pill_touches_the_screen_bounds() {
        for monitor in [monitor(true, 1.0), monitor(false, 2.0)] {
            let unit = monitor.unit_scale();
            // Top-right: the pill's right edge lands exactly on the bound.
            let (x, y) = window_origin_units(&monitor, Anchor::TopRight, PILL, MARGIN, false);
            let pill_right = x + MARGIN * unit + PILL.x * unit;
            assert!(
                (pill_right - monitor.width as f32).abs() < 0.01,
                "flush right edge expected {}, got {pill_right}",
                monitor.width
            );
            // Top: the pill's top edge meets the usable top bound.
            let pill_top = y + MARGIN * unit;
            assert!((pill_top - top_margin_units(&monitor, false)).abs() < 0.01);

            // Top-center: centered either way.
            let (cx, _) = window_origin_units(&monitor, Anchor::TopCenter, PILL, MARGIN, false);
            let center = cx + MARGIN * unit + PILL.x * unit / 2.0;
            assert!((center - monitor.width as f32 / 2.0).abs() < 0.01);
        }
    }

    #[test]
    fn padding_insets_by_exactly_the_extra_amount() {
        let monitor = monitor(true, 2.0);

        let (flush_x, flush_y) =
            window_origin_units(&monitor, Anchor::TopRight, PILL, MARGIN, false);
        let (padded_x, padded_y) =
            window_origin_units(&monitor, Anchor::TopRight, PILL, MARGIN, true);
        assert_eq!(flush_x - padded_x, EXTRA_PADDING_POINTS);
        assert_eq!(padded_y - flush_y, EXTRA_PADDING_POINTS);

        // Center stays centered; only the top edge moves down.
        let (cx, cy) = window_origin_units(&monitor, Anchor::TopCenter, PILL, MARGIN, true);
        let (fx, fy) = window_origin_units(&monitor, Anchor::TopCenter, PILL, MARGIN, false);
        assert_eq!(cx, fx);
        assert_eq!(cy - fy, EXTRA_PADDING_POINTS);
    }

    #[test]
    fn macos_keeps_clear_of_the_menu_bar_when_flush() {
        let macos = monitor(true, 2.0);
        let expected = if cfg!(target_os = "macos") {
            MACOS_MENU_BAR_POINTS
        } else {
            0.0
        };
        assert_eq!(top_margin_units(&macos, false), expected);
        assert_eq!(side_inset_units(&macos, false), 0.0);
    }
}

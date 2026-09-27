//! The eframe application: window lifecycle and native placement for the pill.

use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use bossbar_proto::{Anchor, BarKind, BarState, CollapseMode, Request};
use eframe::egui::{self, ViewportCommand};

use crate::{
    bars::BarStore,
    platform::{self, MonitorInfo},
    state::{Daemon, UiEvent},
    tray::{TrayAction, TrayController},
    view::{self, UiModel},
    waker::Waker,
};

/// Cadence for indeterminate motion (sheen, spinner ring). 20 fps keeps the
/// idle daemon cheap while still reading as motion.
const SPINNER_FRAME: Duration = Duration::from_millis(50);

struct FrameState {
    model: UiModel,
    snapshot: BarState,
    events: Vec<UiEvent>,
    shutdown: bool,
    revision: u64,
    next_repaint: Option<Duration>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ShowPhase {
    /// Window hidden, nothing to draw.
    Dormant,
    /// Window positioned and sized but still invisible; next frame reveals it.
    Positioning,
    Visible,
    /// Fading out before hiding again.
    Hiding,
}

pub struct BossBarApp {
    shared: Arc<Mutex<Daemon>>,
    waker: Waker,
    tray: Option<TrayController>,
    last_frame: Instant,
    started: Instant,
    /// Animated pill size in points (excluding the shadow margin).
    pill_size: egui::Vec2,
    pill_target: egui::Vec2,
    appear: f32,
    phase: ShowPhase,
    monitor: Option<MonitorInfo>,
    last_positioned: Option<(egui::Vec2, Anchor, bool, i32, i32)>,
    /// Logical position last requested from the window manager.
    requested_origin: Option<egui::Pos2>,
    /// How far the OS pushed the window down from that request (macOS keeps
    /// windows clear of the menu bar). Compensated by lifting the pill inside
    /// the window so its visible top edge is flush anyway.
    content_trim: f32,
    #[cfg(windows)]
    hwnd: Option<isize>,
    #[cfg(windows)]
    region: Option<(i32, i32)>,
    #[cfg(target_os = "macos")]
    non_activating: bool,
    last_revision: u64,
}

impl BossBarApp {
    pub fn new(cc: &eframe::CreationContext<'_>, shared: Arc<Mutex<Daemon>>, waker: Waker) -> Self {
        view::setup_fonts(&cc.egui_ctx);
        waker.set_context(cc.egui_ctx.clone());
        let initial = shared
            .lock()
            .map(|daemon| daemon.store.snapshot())
            .unwrap_or_default();
        let tray = match TrayController::new(waker.clone(), &initial) {
            Ok(tray) => {
                tracing::info!("system tray icon is available");
                Some(tray)
            }
            Err(error) => {
                tracing::warn!("system tray icon could not be created: {error:#}");
                None
            }
        };
        Self {
            shared,
            waker,
            tray,
            last_frame: Instant::now(),
            started: Instant::now(),
            pill_size: egui::vec2(view::INITIAL_PILL_SIZE[0], view::INITIAL_PILL_SIZE[1]),
            pill_target: egui::vec2(view::INITIAL_PILL_SIZE[0], view::INITIAL_PILL_SIZE[1]),
            appear: 0.0,
            phase: ShowPhase::Dormant,
            monitor: None,
            last_positioned: None,
            requested_origin: None,
            content_trim: 0.0,
            #[cfg(windows)]
            hwnd: None,
            #[cfg(windows)]
            region: None,
            #[cfg(target_os = "macos")]
            non_activating: false,
            last_revision: 0,
        }
    }

    fn request(&self, request: Request) {
        if let Ok(mut daemon) = self.shared.lock() {
            daemon.apply(&request);
        }
        self.waker.wake();
    }

    fn apply_tray(&mut self, action: TrayAction) {
        let request = match action {
            TrayAction::ToggleVisible => Request::SetVisible { value: None },
            TrayAction::ToggleCollapse => Request::Collapse { value: None },
            TrayAction::SetAnchor(anchor) => Request::SetPosition { anchor },
            TrayAction::SetCollapseMode(mode) => Request::SetCollapseMode { mode },
            TrayAction::TogglePadding => Request::SetPadding { value: None },
            TrayAction::ClearAll => Request::Clear,
            TrayAction::Quit => Request::Shutdown,
        };
        self.request(request);
    }

    /// Re-resolves the monitor under the cursor and moves the pill there.
    fn reposition(&mut self, ctx: &egui::Context, pill_size: egui::Vec2) {
        let monitor = platform::active_monitor().or(self.monitor);
        let Some(monitor) = monitor else {
            tracing::warn!("no monitor available to position the pill");
            return;
        };
        self.monitor = Some(monitor);
        let (anchor, padding) = self
            .shared
            .lock()
            .map(|daemon| (daemon.store.anchor, daemon.store.padding))
            .unwrap_or_default();
        self.requested_origin = Some(platform::place_window(
            ctx,
            monitor,
            anchor,
            pill_size,
            view::SHADOW_MARGIN,
            padding,
        ));
        self.last_positioned = Some((pill_size, anchor, padding, monitor.x, monitor.y));
    }

    /// Applies the native window size (and Windows shape), then re-anchors if
    /// the animated size moved the centered/right edge.
    fn send_window_size(&mut self, ctx: &egui::Context, pill_size: egui::Vec2) {
        let window_size = pill_size + egui::Vec2::splat(view::SHADOW_MARGIN * 2.0);
        ctx.send_viewport_cmd(ViewportCommand::InnerSize(window_size));

        #[cfg(windows)]
        {
            if let Some(hwnd) = self.hwnd {
                platform::apply_window_region(
                    hwnd,
                    window_size,
                    ctx.pixels_per_point(),
                    view::pill_radius(pill_size),
                    &mut self.region,
                );
            }
        }

        let (anchor, padding) = self
            .shared
            .lock()
            .map(|daemon| (daemon.store.anchor, daemon.store.padding))
            .unwrap_or_default();
        if let Some(monitor) = self.monitor {
            // Anchoring follows the pill size, not the shadow-inflated window.
            let key = (pill_size, anchor, padding, monitor.x, monitor.y);
            if self.last_positioned != Some(key) {
                self.requested_origin = Some(platform::place_window(
                    ctx,
                    monitor,
                    anchor,
                    pill_size,
                    view::SHADOW_MARGIN,
                    padding,
                ));
                self.last_positioned = Some(key);
            }
        }
    }

    fn drain_state(&mut self, now: Instant, dt: f32) -> FrameState {
        let Ok(mut daemon) = self.shared.lock() else {
            return FrameState {
                model: UiModel::collect(&BarStore::default()),
                snapshot: BarState::default(),
                events: Vec::new(),
                shutdown: false,
                revision: self.last_revision,
                next_repaint: None,
            };
        };
        let events = daemon.take_events();
        daemon.store.advance(now, dt);
        let model = UiModel::collect(&daemon.store);
        let snapshot = daemon.store.snapshot();
        let next_repaint = daemon.store.next_repaint(now);
        FrameState {
            model,
            snapshot,
            shutdown: daemon.is_shutdown(),
            revision: daemon.revision,
            events,
            next_repaint,
        }
    }

    /// Compares the requested window origin with the actual one and returns
    /// how far the pill must be lifted inside the window to stay flush.
    fn update_content_trim(&mut self, ctx: &egui::Context) -> f32 {
        let Some(requested) = self.requested_origin else {
            return self.content_trim;
        };
        let actual = ctx.input(|input| input.viewport().outer_rect.map(|rect| rect.min));
        if let Some(actual) = actual {
            self.content_trim = (actual.y - requested.y).clamp(0.0, view::SHADOW_MARGIN);
        }
        self.content_trim
    }

    fn spinner_phase(&self) -> f32 {
        (self.started.elapsed().as_secs_f32() * 0.85).fract()
    }

    fn render(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame, model: &UiModel) {
        #[cfg(target_os = "macos")]
        if !self.non_activating {
            self.non_activating = platform::macos_prevent_activation(frame);
            if self.non_activating {
                tracing::debug!("pill window marked non-activating");
            }
        }
        let _ = frame;

        let pill_size = self.pill_size;
        let appear = self.appear;
        let spinner_phase = self.spinner_phase();
        let trim = self.update_content_trim(ctx);
        let mut requests: Vec<Request> = Vec::new();
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ctx, |ui| {
                view::render_pill(
                    ui,
                    model,
                    pill_size,
                    trim,
                    appear,
                    spinner_phase,
                    &mut |request| {
                        requests.push(request);
                    },
                );
            });
        for request in requests {
            self.request(request);
        }
    }
}

impl eframe::App for BossBarApp {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0, 0.0, 0.0, 0.0]
    }

    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        let now = Instant::now();
        let dt = now
            .duration_since(self.last_frame)
            .as_secs_f32()
            .clamp(0.0001, 0.1);
        self.last_frame = now;

        // The native handle exists from the first frame on; the waker needs it
        // early so it can make the window paintable again after a hide.
        #[cfg(windows)]
        if self.hwnd.is_none() {
            self.hwnd = platform::window_hwnd(frame);
            if let Some(hwnd) = self.hwnd {
                self.waker.set_hwnd(hwnd);
            }
        }

        while let Some(action) = self.tray.as_ref().and_then(TrayController::try_recv) {
            self.apply_tray(action);
        }

        let state = self.drain_state(now, dt);
        if state.shutdown {
            // `ViewportCommand::Close` is observed by eframe on the next
            // frame, so keep frames coming until the event loop exits; the
            // hidden suppression would otherwise swallow them.
            tracing::debug!("shutdown requested; closing the viewport");
            ctx.send_viewport_cmd(ViewportCommand::Close);
            ctx.request_repaint();
            return;
        }
        if state.events.contains(&UiEvent::Reposition) {
            self.reposition(ctx, self.pill_target);
        }

        let should_show = state.model.visible && !state.model.rows.is_empty();
        let target = view::target_size(ctx, &state.model, self.pill_size);
        self.pill_target = target;

        match self.phase {
            ShowPhase::Dormant => {
                if should_show {
                    self.phase = ShowPhase::Positioning;
                    self.appear = 1.0;
                    self.pill_size = target;
                    self.reposition(ctx, target);
                    self.send_window_size(ctx, target);
                    ctx.request_repaint();
                    tracing::debug!(?target, "pill appearing");
                } else {
                    // A wake may have shown the window so the event loop could
                    // deliver this very frame (see `Waker`); with nothing to
                    // display, park it again.
                    #[cfg(windows)]
                    if let Some(hwnd) = self.hwnd {
                        platform::hide_if_visible(hwnd);
                    }
                }
            }
            ShowPhase::Positioning => {
                if should_show {
                    self.send_window_size(ctx, target);
                    ctx.send_viewport_cmd(ViewportCommand::Visible(true));
                    self.phase = ShowPhase::Visible;
                    tracing::debug!("pill revealed");
                } else {
                    self.phase = ShowPhase::Dormant;
                }
            }
            ShowPhase::Visible => {
                if !should_show {
                    self.phase = ShowPhase::Hiding;
                }
            }
            ShowPhase::Hiding => {
                if should_show {
                    self.phase = ShowPhase::Positioning;
                    self.appear = 1.0;
                    self.pill_size = target;
                    self.reposition(ctx, target);
                    self.send_window_size(ctx, target);
                    ctx.request_repaint();
                }
            }
        }

        let visible = matches!(self.phase, ShowPhase::Visible | ShowPhase::Hiding);
        let mut animating = false;
        if visible {
            let delta = self.pill_target - self.pill_size;
            if delta.length() > 0.2 {
                self.pill_size += delta * (1.0 - (-16.0 * dt).exp());
                animating = true;
            } else if delta != egui::Vec2::ZERO {
                self.pill_size = self.pill_target;
                animating = true;
            }
            let appear_target = if should_show { 1.0 } else { 0.0 };
            let appear_delta = appear_target - self.appear;
            if appear_delta.abs() > 0.005 {
                self.appear += appear_delta * (1.0 - (-18.0 * dt).exp());
                animating = true;
            } else {
                self.appear = appear_target;
            }
            if animating {
                self.send_window_size(ctx, self.pill_size);
            }
        }

        if self.phase == ShowPhase::Hiding && self.appear <= 0.02 {
            ctx.send_viewport_cmd(ViewportCommand::Visible(false));
            self.phase = ShowPhase::Dormant;
            tracing::debug!("pill hidden; waiting for the next bar");
        }

        if visible {
            self.render(ctx, frame, &state.model);
        }

        let indeterminate = state
            .model
            .rows
            .iter()
            .any(|row| matches!(row.kind, BarKind::Indeterminate));
        let spinner = indeterminate
            || (state.model.uses_collapsed_layout()
                && state.model.collapse_mode == CollapseMode::Compact
                && state.model.aggregate.is_none());
        let mut delay: Option<Duration> = None;
        let mut propose = |candidate: Duration| {
            delay = Some(delay.map_or(candidate, |current: Duration| current.min(candidate)));
        };
        if animating {
            propose(Duration::from_millis(16));
        }
        if let Some(store_delay) = state.next_repaint {
            propose(store_delay.max(Duration::from_millis(16)));
        }
        if spinner && visible {
            propose(SPINNER_FRAME);
        }
        if let Some(delay) = delay {
            tracing::trace!(?delay, "scheduling repaint");
            ctx.request_repaint_after(delay);
        }

        if state.revision != self.last_revision {
            self.last_revision = state.revision;
            if let Some(tray) = &self.tray {
                tray.sync(&state.snapshot);
            }
        }
    }
}

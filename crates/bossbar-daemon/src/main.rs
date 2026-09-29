//! bossbar-daemon: a GUI-less background process serving the pill overlay
//! and its loopback IPC API.
//!
//! Lifecycle: acquire a per-user lock, start the IPC listener, run eframe on
//! a custom event loop with a hidden window. Bars are created through the
//! `bossbar` CLI (or the tray menu); the window only exists while bars do.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod bars;
mod event_loop;
mod ipc;
mod platform;
mod state;
mod tray;
mod view;
mod waker;

use std::{
    fs::{self, OpenOptions},
    sync::{Arc, Mutex},
};

use anyhow::{Context as _, Result};
use eframe::NativeOptions;
use tracing_subscriber::EnvFilter;

use crate::{app::BossBarApp, state::Daemon, waker::Waker};

/// Log files above this size are truncated on the next start.
const MAX_LOG_BYTES: u64 = 8 * 1024 * 1024;

fn main() {
    if let Err(error) = run() {
        eprintln!("bossbar-daemon: {error:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    init_logging();

    let Some(state_dir) = bossbar_proto::state_dir() else {
        anyhow::bail!("cannot resolve the bossbar state directory");
    };
    fs::create_dir_all(&state_dir).context("create bossbar state directory")?;

    // One daemon per user session. Concurrent launches are a no-op: the CLI
    // and login items may both try to start us.
    let lock_path = bossbar_proto::lock_path().context("resolve bossbar lock path")?;
    let instance_lock = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&lock_path)
        .context("open bossbar instance lock")?;
    if let Err(error) = fs2::FileExt::try_lock_exclusive(&instance_lock) {
        let already_running = error.kind() == std::io::ErrorKind::WouldBlock
            || matches!(error.raw_os_error(), Some(32 | 33));
        if already_running {
            tracing::info!("another bossbar daemon is already running; exiting");
            return Ok(());
        }
        return Err(error).context("lock bossbar instance");
    }

    let waker = Waker::default();
    let shared = Arc::new(Mutex::new(Daemon::default()));
    let info = ipc::serve(Arc::clone(&shared), waker.clone()).context("start bossbar IPC")?;
    tracing::info!(pid = info.pid, port = info.port, "bossbar daemon started");

    let result = event_loop::run(
        "bossbar",
        native_options(),
        Box::new(move |cc| Ok(Box::new(BossBarApp::new(cc, shared, waker)))),
    );

    ipc::clear_info(&info);
    drop(instance_lock);
    tracing::info!("bossbar daemon stopped");
    result.map_err(|error| anyhow::anyhow!("eframe event loop failed: {error}"))
}

fn native_options() -> NativeOptions {
    let mut options = NativeOptions {
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };
    // Repaint continuously only while something is moving and let the
    // compositor pace those frames to the active display. Keeping this
    // explicit prevents a dependency default from reintroducing a fixed or
    // tearing present mode.
    options.wgpu_options.surface.present_mode = wgpu::PresentMode::AutoVsync;
    options.viewport = options
        .viewport
        .with_title("bossbar")
        .with_icon(load_app_icon())
        .with_decorations(false)
        .with_transparent(true)
        .with_resizable(false)
        .with_active(false)
        .with_taskbar(false)
        .with_window_level(egui::WindowLevel::AlwaysOnTop)
        .with_inner_size([
            view::INITIAL_PILL_SIZE[0] + view::SHADOW_MARGIN * 2.0,
            view::INITIAL_PILL_SIZE[1] + view::SHADOW_MARGIN * 2.0,
        ])
        .with_position([-20_000.0, -20_000.0])
        .with_visible(false);
    #[cfg(target_os = "macos")]
    {
        options.viewport = options.viewport.with_has_shadow(false);
    }
    options
}

fn load_app_icon() -> egui::IconData {
    let image = image::load_from_memory(include_bytes!("../assets/icon.png"))
        .expect("app icon asset")
        .into_rgba8();
    let (width, height) = image.dimensions();
    egui::IconData {
        rgba: image.into_raw(),
        width,
        height,
    }
}

fn init_logging() {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("warn,bossbar_daemon=debug"));
    if let Some(path) = bossbar_proto::log_path() {
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        if fs::metadata(&path).is_ok_and(|metadata| metadata.len() > MAX_LOG_BYTES) {
            let _ = fs::remove_file(&path);
        }
        if let Ok(file) = OpenOptions::new().create(true).append(true).open(&path) {
            if tracing_subscriber::fmt()
                .with_env_filter(filter.clone())
                .with_ansi(false)
                .with_writer(std::sync::Mutex::new(file))
                .try_init()
                .is_ok()
            {
                tracing::info!(path = %path.display(), "logging to file");
                return;
            }
        }
    }
    let _ = tracing_subscriber::fmt().with_env_filter(filter).try_init();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_renderer_is_vsync_paced() {
        let options = native_options();
        assert_eq!(
            options.wgpu_options.surface.present_mode,
            wgpu::PresentMode::AutoVsync
        );
    }
}

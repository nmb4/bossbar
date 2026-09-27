//! System-tray integration: the daemon's always-present entry point.
//!
//! Adapted from wire-app's tray controller: the OS resources live in
//! [`TrayIcon`] (with a stable GUID on Windows), events are forwarded over a
//! channel, and the app drains them on the UI thread.

use std::sync::mpsc::{self, Receiver};

use anyhow::{Context as _, Result};
use bossbar_proto::{Anchor, BarState};
use tray_icon::{
    menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem},
    Icon, TrayIcon, TrayIconBuilder,
};

use crate::waker::Waker;

const SHOW_ID: &str = "bossbar.tray.show";
const COLLAPSE_ID: &str = "bossbar.tray.collapse";
const POS_CENTER_ID: &str = "bossbar.tray.position.center";
const POS_RIGHT_ID: &str = "bossbar.tray.position.right";
const PADDING_ID: &str = "bossbar.tray.padding";
const CLEAR_ID: &str = "bossbar.tray.clear";
const QUIT_ID: &str = "bossbar.tray.quit";
const TRAY_GUID: u128 = 0x7E2C_1F7A_9B44_4D19_A3E6_0C58_2B77_D9F4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayAction {
    ToggleVisible,
    ToggleCollapse,
    SetAnchor(Anchor),
    TogglePadding,
    ClearAll,
    Quit,
}

pub struct TrayController {
    // tray-icon owns the OS resources through this handle; dropping it removes
    // the icon.
    _icon: TrayIcon,
    signal_rx: Receiver<TrayAction>,
    show_item: CheckMenuItem,
    collapse_item: CheckMenuItem,
    pos_center_item: CheckMenuItem,
    pos_right_item: CheckMenuItem,
    padding_item: CheckMenuItem,
}

impl TrayController {
    /// Must be built after the native event loop is running (macOS
    /// requirement, and safest on Windows).
    pub fn new(waker: Waker, initial: &BarState) -> Result<Self> {
        let menu = Menu::new();
        let show_item = CheckMenuItem::with_id(SHOW_ID, "Show bars", true, initial.visible, None);
        let collapse_item =
            CheckMenuItem::with_id(COLLAPSE_ID, "Collapse", true, initial.collapsed, None);
        let pos_center_item = CheckMenuItem::with_id(
            POS_CENTER_ID,
            Anchor::TopCenter.label(),
            true,
            initial.anchor == Anchor::TopCenter,
            None,
        );
        let pos_right_item = CheckMenuItem::with_id(
            POS_RIGHT_ID,
            Anchor::TopRight.label(),
            true,
            initial.anchor == Anchor::TopRight,
            None,
        );
        let position_root = tray_icon::menu::Submenu::with_id_and_items(
            "bossbar.tray.position",
            "Position",
            true,
            &[&pos_center_item, &pos_right_item],
        )
        .context("build tray position submenu")?;
        let padding_item =
            CheckMenuItem::with_id(PADDING_ID, "Extra padding", true, initial.padding, None);
        let clear_item = MenuItem::with_id(CLEAR_ID, "Clear all bars", true, None);
        let quit_item = MenuItem::with_id(QUIT_ID, "Quit bossbar", true, None);
        menu.append_items(&[
            &show_item,
            &collapse_item,
            &position_root,
            &padding_item,
            &PredefinedMenuItem::separator(),
            &clear_item,
            &PredefinedMenuItem::separator(),
            &quit_item,
        ])
        .context("build tray menu")?;

        let (signal_tx, signal_rx) = mpsc::channel();
        let icon = load_icon()?;
        let tray = TrayIconBuilder::new()
            .with_id("bossbar")
            .with_guid(TRAY_GUID)
            .with_icon(icon)
            .with_icon_as_template(cfg!(target_os = "macos"))
            .with_menu(Box::new(menu))
            // Any click opens the menu; there is no separate click action.
            .with_menu_on_left_click(true)
            .with_tooltip("bossbar")
            .build()
            .context("create system tray icon")?;

        let menu_tx = signal_tx;
        let menu_waker = waker;
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            let action = match event.id.as_ref() {
                SHOW_ID => Some(TrayAction::ToggleVisible),
                COLLAPSE_ID => Some(TrayAction::ToggleCollapse),
                POS_CENTER_ID => Some(TrayAction::SetAnchor(Anchor::TopCenter)),
                POS_RIGHT_ID => Some(TrayAction::SetAnchor(Anchor::TopRight)),
                PADDING_ID => Some(TrayAction::TogglePadding),
                CLEAR_ID => Some(TrayAction::ClearAll),
                QUIT_ID => Some(TrayAction::Quit),
                _ => None,
            };
            if let Some(action) = action {
                tracing::debug!(?action, "tray action");
                let _ = menu_tx.send(action);
                menu_waker.wake();
            }
        }));

        Ok(Self {
            _icon: tray,
            signal_rx,
            show_item,
            collapse_item,
            pos_center_item,
            pos_right_item,
            padding_item,
        })
    }

    pub fn try_recv(&self) -> Option<TrayAction> {
        self.signal_rx.try_recv().ok()
    }

    /// Mirrors daemon state into the checkmarks. Cheap to call every frame;
    /// only touches items whose value changed.
    pub fn sync(&self, state: &BarState) {
        let collapse = state.collapsed && state.bars.len() >= 2;
        set_checked(&self.show_item, state.visible);
        set_checked(&self.collapse_item, collapse);
        set_checked(&self.pos_center_item, state.anchor == Anchor::TopCenter);
        set_checked(&self.pos_right_item, state.anchor == Anchor::TopRight);
        set_checked(&self.padding_item, state.padding);
    }
}

fn set_checked(item: &CheckMenuItem, checked: bool) {
    if item.is_checked() != checked {
        item.set_checked(checked);
    }
}

fn load_icon() -> Result<Icon> {
    let image = image::load_from_memory(include_bytes!("../assets/tray-template.png"))
        .context("load tray icon asset")?
        .into_rgba8();
    let (width, height) = image.dimensions();
    Icon::from_rgba(image.into_raw(), width, height).context("convert tray icon asset")
}

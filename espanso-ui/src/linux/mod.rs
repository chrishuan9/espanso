/*
 * This file is part of espanso.
 *
 * Copyright (C) 2019-2021 Federico Terzi
 *
 * espanso is free software: you can redistribute it and/or modify
 * it under the terms of the GNU General Public License as published by
 * the Free Software Foundation, either version 3 of the License, or
 * (at your option) any later version.
 *
 * espanso is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
 * GNU General Public License for more details.
 *
 * You should have received a copy of the GNU General Public License
 * along with espanso.  If not, see <https://www.gnu.org/licenses/>.
 */

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Result;
use crossbeam::{
    channel::{bounded, unbounded, Receiver, Sender},
    select,
};
use image::GenericImageView;
use ksni::{
    blocking::TrayMethods,
    menu::{StandardItem, SubMenu},
    Icon as KsniIcon, MenuItem as KsniMenuItem,
};
use log::{debug, error, warn};
use notify_rust::Notification;

use crate::{
    event::UIEvent,
    icons::TrayIcon,
    menu::{Menu, MenuItem as GenericMenuItem},
    UIEventCallback, UIEventLoop, UIRemote,
};

type SharedCallback = Arc<Mutex<Option<UIEventCallback>>>;
type SharedHandle = Arc<Mutex<Option<ksni::blocking::Handle<LinuxTray>>>>;

// The systemd --user unit name managed by `espanso service register`, see
// `LINUX_SERVICE_NAME` in espanso/src/cli/service/linux.rs. Duplicated here as a
// literal because espanso-ui cannot depend on the espanso binary crate.
const SYSTEMD_SERVICE_NAME: &str = "espanso";

// Bounded wait for the engine to compute the context menu in response to a
// TrayIconClick, so a stuck engine can't wedge ksni's D-Bus thread forever.
const MENU_ROUND_TRIP_TIMEOUT: Duration = Duration::from_secs(2);

pub struct LinuxUIOptions {
    pub show_icon: bool,
    pub icon_paths: Vec<(TrayIcon, String)>,
    pub notification_icon_path: String,
}

pub fn create(options: LinuxUIOptions) -> (LinuxRemote, LinuxEventLoop) {
    let LinuxUIOptions {
        show_icon,
        icon_paths,
        notification_icon_path,
    } = options;

    let (exit_tx, exit_rx) = unbounded();
    let (menu_ready_tx, menu_ready_rx) = bounded(1);
    let callback: SharedCallback = Arc::new(Mutex::new(None));
    let handle: SharedHandle = Arc::new(Mutex::new(None));
    let icons = decode_icons(&icon_paths);

    let remote = LinuxRemote::new(
        exit_tx,
        notification_icon_path,
        handle.clone(),
        menu_ready_tx,
    );
    let eventloop = LinuxEventLoop::new(exit_rx, callback, handle, menu_ready_rx, icons, show_icon);

    (remote, eventloop)
}

fn decode_icons(icon_paths: &[(TrayIcon, String)]) -> HashMap<TrayIcon, KsniIcon> {
    let mut icons = HashMap::new();
    for (tray_icon, path) in icon_paths {
        match decode_icon(path) {
            Ok(icon) => {
                icons.insert(tray_icon.clone(), icon);
            }
            Err(error) => {
                error!("unable to decode tray icon at '{path}': {error}");
            }
        }
    }
    icons
}

// StatusNotifierItem wants raw ARGB32 pixel data (network byte order) rather than
// a path or icon-theme name, since espanso doesn't reliably install its icons into
// the system hicolor theme across all install methods (cargo install, AppImage).
fn decode_icon(path: &str) -> Result<KsniIcon> {
    let bytes = std::fs::read(path)?;
    let img = image::load_from_memory_with_format(&bytes, image::ImageFormat::Png)?;
    let (width, height) = img.dimensions();
    let mut data = img.into_rgba8().into_vec();
    for pixel in data.chunks_exact_mut(4) {
        pixel.rotate_right(1); // RGBA -> ARGB
    }
    Ok(KsniIcon {
        width: width as i32,
        height: height as i32,
        data,
    })
}

fn emit_event(callback: &SharedCallback, event: UIEvent) {
    let guard = callback.lock().unwrap();
    if let Some(callback) = guard.as_ref() {
        callback(event);
    } else {
        warn!("tray UI event callback not yet initialized, dropping event {event:?}");
    }
}

pub struct LinuxRemote {
    tx: Sender<()>,
    notification_icon_path: String,
    handle: SharedHandle,
    menu_ready_tx: Sender<Menu>,
}

impl LinuxRemote {
    fn new(
        tx: Sender<()>,
        notification_icon_path: String,
        handle: SharedHandle,
        menu_ready_tx: Sender<Menu>,
    ) -> Self {
        Self {
            tx,
            notification_icon_path,
            handle,
            menu_ready_tx,
        }
    }

    pub fn stop(&self) -> anyhow::Result<()> {
        Ok(self.tx.send(())?)
    }
}

impl UIRemote for LinuxRemote {
    fn update_tray_icon(&self, icon: TrayIcon) {
        // SystemDisabled is currently unreachable on Linux (secure_input.rs is a
        // NOOP outside macOS), but map it defensively so the icon set stays exhaustive.
        let icon = match icon {
            TrayIcon::SystemDisabled => TrayIcon::Disabled,
            other => other,
        };

        let guard = self.handle.lock().unwrap();
        if let Some(handle) = guard.as_ref() {
            let _ = handle.update(|tray: &mut LinuxTray| tray.current_icon = icon);
        }
    }

    fn show_notification(&self, message: &str) {
        if let Err(error) = Notification::new()
            .summary("Espanso")
            .body(message)
            .icon(&self.notification_icon_path)
            .show()
        {
            error!("Unable to show notification: {}", error);
        }
    }

    fn show_context_menu(&self, menu: &Menu) {
        if self.menu_ready_tx.try_send(menu.clone()).is_err() {
            debug!("no pending tray menu request, dropping computed context menu");
        }
    }

    fn exit(&self) {
        if let Some(handle) = self.handle.lock().unwrap().take() {
            handle.shutdown().wait();
        }
        self.stop()
            .expect("unable to send termination signal to ui eventloop");
    }
}

pub struct LinuxEventLoop {
    rx: Receiver<()>,
    callback: SharedCallback,
    handle: SharedHandle,
    menu_ready_rx: Option<Receiver<Menu>>,
    icons: HashMap<TrayIcon, KsniIcon>,
    show_icon: bool,
}

impl LinuxEventLoop {
    fn new(
        rx: Receiver<()>,
        callback: SharedCallback,
        handle: SharedHandle,
        menu_ready_rx: Receiver<Menu>,
        icons: HashMap<TrayIcon, KsniIcon>,
        show_icon: bool,
    ) -> Self {
        Self {
            rx,
            callback,
            handle,
            menu_ready_rx: Some(menu_ready_rx),
            icons,
            show_icon,
        }
    }
}

impl UIEventLoop for LinuxEventLoop {
    fn initialize(&mut self) -> Result<()> {
        if !self.show_icon {
            return Ok(());
        }

        let menu_ready_rx = self
            .menu_ready_rx
            .take()
            .expect("LinuxEventLoop::initialize should only be called once");

        let tray = LinuxTray {
            icons: self.icons.clone(),
            current_icon: TrayIcon::Normal,
            callback: self.callback.clone(),
            menu_ready_rx,
            cached_menu: RefCell::new(Vec::new()),
        };

        match tray.spawn() {
            Ok(handle) => {
                *self.handle.lock().unwrap() = Some(handle);
            }
            Err(error) => {
                // Not fatal: espanso keeps running headless, matching how mac/win
                // degrade if their native tray fails to initialize.
                error!(
                    "unable to start the tray icon service, it will not be shown: {error}. Make \
                     sure a StatusNotifierItem host is installed and enabled (on GNOME, the \
                     \"AppIndicator and KStatusNotifierItem Support\" extension is required)."
                );
            }
        }

        Ok(())
    }

    fn run(&self, callback: crate::UIEventCallback) -> Result<()> {
        *self.callback.lock().unwrap() = Some(callback);

        loop {
            select! {
                recv(self.rx) -> result => {
                    match result {
                        Ok(()) => {
                            // remote.exit() called
                            return Ok(());
                        }
                        Err(error) => {
                            error!("Unable to block the LinuxEventLoop: {}", error);
                            return Err(error.into());
                        }
                    }
                },
                default(Duration::from_secs(1)) => {
                    emit_event(&self.callback, UIEvent::Heartbeat);
                }
            }
        }
    }
}

pub struct LinuxTray {
    icons: HashMap<TrayIcon, KsniIcon>,
    current_icon: TrayIcon,
    callback: SharedCallback,
    menu_ready_rx: Receiver<Menu>,
    cached_menu: RefCell<Vec<KsniMenuItem<LinuxTray>>>,
}

impl ksni::Tray for LinuxTray {
    // Our tray only ever offers a menu, there's no separate "activate" action,
    // so make left-click open the menu instead of doing nothing.
    const MENU_ON_ACTIVATE: bool = true;

    fn id(&self) -> String {
        "espanso".into()
    }

    fn title(&self) -> String {
        "Espanso".into()
    }

    fn icon_pixmap(&self) -> Vec<KsniIcon> {
        self.icons
            .get(&self.current_icon)
            .cloned()
            .into_iter()
            .collect()
    }

    fn menu_about_to_show(&mut self) {
        // Drain any stale response left over from a previous, timed-out round trip.
        while self.menu_ready_rx.try_recv().is_ok() {}

        emit_event(&self.callback, UIEvent::TrayIconClick);

        let items = if let Ok(menu) = self.menu_ready_rx.recv_timeout(MENU_ROUND_TRIP_TIMEOUT) {
            build_menu(menu, &self.callback)
        } else {
            warn!("timed out waiting for the engine to build the context menu");
            fallback_menu(&self.callback)
        };

        *self.cached_menu.borrow_mut() = items;
    }

    fn menu(&self) -> Vec<KsniMenuItem<Self>> {
        std::mem::take(&mut *self.cached_menu.borrow_mut())
    }
}

fn build_menu(menu: Menu, callback: &SharedCallback) -> Vec<KsniMenuItem<LinuxTray>> {
    let mut items = systemd_status_items();
    items.extend(convert_items(menu.items, callback));
    items
}

fn convert_items(
    items: Vec<GenericMenuItem>,
    callback: &SharedCallback,
) -> Vec<KsniMenuItem<LinuxTray>> {
    items
        .into_iter()
        .map(|item| convert_item(item, callback))
        .collect()
}

fn convert_item(item: GenericMenuItem, callback: &SharedCallback) -> KsniMenuItem<LinuxTray> {
    match item {
        GenericMenuItem::Simple(simple) => {
            let callback = callback.clone();
            let id = simple.id;
            StandardItem {
                label: simple.label,
                activate: Box::new(move |_tray: &mut LinuxTray| {
                    emit_event(&callback, UIEvent::ContextMenuClick(id));
                }),
                ..Default::default()
            }
            .into()
        }
        GenericMenuItem::Sub(sub) => SubMenu {
            label: sub.label,
            submenu: convert_items(sub.items, callback),
            ..Default::default()
        }
        .into(),
        GenericMenuItem::Separator => KsniMenuItem::Separator,
    }
}

// Degraded-mode menu shown if the engine doesn't answer a TrayIconClick in time.
fn fallback_menu(callback: &SharedCallback) -> Vec<KsniMenuItem<LinuxTray>> {
    let callback = callback.clone();
    vec![StandardItem {
        label: "Exit espanso".into(),
        // Hardcoded id 0: matches CONTEXT_ITEM_EXIT in
        // espanso-engine/src/process/middleware/context_menu.rs. Only used here,
        // as a last resort when the normal engine round trip failed.
        activate: Box::new(move |_tray: &mut LinuxTray| {
            emit_event(&callback, UIEvent::ContextMenuClick(0));
        }),
        ..Default::default()
    }
    .into()]
}

// Linux-only addition beyond mac/Windows parity: since the tray runs inside the
// worker, which is itself a child of the systemd --user service, its lifecycle can
// silently diverge from the service's. Surface live status + a restart action.
fn systemd_status_items() -> Vec<KsniMenuItem<LinuxTray>> {
    let status = systemd_service_status();
    vec![
        StandardItem {
            label: format!("Service: {status}"),
            enabled: false,
            ..Default::default()
        }
        .into(),
        StandardItem {
            label: "Restart espanso service".into(),
            activate: Box::new(|_tray: &mut LinuxTray| {
                // Never wait on this command: `systemctl restart` SIGTERMs the whole
                // unit's cgroup, including the worker process issuing the call.
                if let Err(error) = std::process::Command::new("systemctl")
                    .args(["--user", "restart", SYSTEMD_SERVICE_NAME])
                    .spawn()
                {
                    error!(
                        "failed to run 'systemctl --user restart {SYSTEMD_SERVICE_NAME}': {error}"
                    );
                }
            }),
            ..Default::default()
        }
        .into(),
        KsniMenuItem::Separator,
    ]
}

fn systemd_service_status() -> String {
    match std::process::Command::new("systemctl")
        .args(["--user", "is-active", SYSTEMD_SERVICE_NAME])
        .output()
    {
        Ok(output) => match String::from_utf8_lossy(&output.stdout).trim() {
            "active" => "● running".to_string(),
            "failed" => "● failed".to_string(),
            "inactive" => "● stopped".to_string(),
            "activating" => "● starting…".to_string(),
            "deactivating" => "● stopping…".to_string(),
            other if !other.is_empty() => other.to_string(),
            _ => "unknown".to_string(),
        },
        Err(error) => {
            debug!("unable to query systemd status: {error}");
            "unknown (systemctl not available)".to_string()
        }
    }
}

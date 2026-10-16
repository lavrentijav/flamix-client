//! Linux tray backend: StatusNotifierItem over D-Bus via `ksni` (no GTK build dependency).
//!
//! The service needs a D-Bus *session* bus. On a headless host (or when no
//! `org.kde.StatusNotifierWatcher` is around) creation fails and we return `None`, so the
//! app simply starts without a tray. The service runs on the app's tokio runtime.

use super::icon;
use super::{MenuEntry, TrayCommand, TraySink, TrayState};
use ksni::menu::{CheckmarkItem, StandardItem};
use ksni::TrayMethods;
use std::sync::Arc;

pub fn create(
    handler: Arc<dyn Fn(TrayCommand) + Send + Sync>,
    initial: &TrayState,
    rt: Option<tokio::runtime::Handle>,
) -> Option<Box<dyn TraySink>> {
    let rt = rt?;
    if !session_bus_available() {
        tracing::info!("no D-Bus session bus; system tray disabled");
        return None;
    }
    let normal = icon::load(false)?;
    let grey = icon::load(true)?;
    let tray = LinuxTray {
        state: initial.clone(),
        handler,
        normal: to_ksni_icon(&normal),
        grey: to_ksni_icon(&grey),
    };
    // `spawn` registers the SNI object and connects to the watcher; failures (no watcher,
    // D-Bus refused, ...) are not fatal for the app.
    match rt.block_on(tray.spawn()) {
        Ok(handle) => Some(Box::new(LinuxSink { handle, rt })),
        Err(err) => {
            tracing::info!(%err, "could not register the system tray");
            None
        }
    }
}

fn to_ksni_icon(img: &icon::Rgba) -> ksni::Icon {
    ksni::Icon {
        width: img.width as i32,
        height: img.height as i32,
        data: icon::rgba_to_argb(&img.data),
    }
}

/// Is there a usable session bus? Cheap pre-check before handing control to zbus.
fn session_bus_available() -> bool {
    if std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_some() {
        return true;
    }
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(|dir| std::path::Path::new(&dir).join("bus").exists())
        .unwrap_or(false)
}

struct LinuxTray {
    state: TrayState,
    handler: Arc<dyn Fn(TrayCommand) + Send + Sync>,
    normal: ksni::Icon,
    grey: ksni::Icon,
}

impl ksni::Tray for LinuxTray {
    fn id(&self) -> String {
        "flamix".into()
    }

    fn title(&self) -> String {
        super::tooltip(&self.state)
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        if self.state.connected {
            vec![self.normal.clone()]
        } else {
            vec![self.grey.clone()]
        }
    }

    /// Left click / activate shows the window.
    fn activate(&mut self, _x: i32, _y: i32) {
        (self.handler)(TrayCommand::Show);
    }

    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        let mut items: Vec<ksni::MenuItem<Self>> = Vec::with_capacity(9);
        for entry in super::menu_entries(&self.state) {
            match entry {
                MenuEntry::Separator => items.push(ksni::MenuItem::Separator),
                MenuEntry::Item {
                    label,
                    enabled,
                    checked,
                    command,
                } => {
                    let handler = self.handler.clone();
                    let activate = Box::new(move |_: &mut LinuxTray| {
                        if let Some(command) = command {
                            handler(command);
                        }
                    });
                    match checked {
                        Some(checked) => items.push(
                            CheckmarkItem {
                                label,
                                enabled,
                                checked,
                                activate,
                                ..Default::default()
                            }
                            .into(),
                        ),
                        None => items.push(
                            StandardItem {
                                label,
                                enabled,
                                activate,
                                ..Default::default()
                            }
                            .into(),
                        ),
                    }
                }
            }
        }
        items
    }
}

struct LinuxSink {
    handle: ksni::Handle<LinuxTray>,
    rt: tokio::runtime::Handle,
}

impl TraySink for LinuxSink {
    fn apply(&self, state: &TrayState) {
        let handle = self.handle.clone();
        let state = state.clone();
        self.rt.spawn(async move {
            let _ = handle
                .update(move |tray: &mut LinuxTray| tray.state = state)
                .await;
        });
    }
}

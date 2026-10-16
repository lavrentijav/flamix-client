//! Windows/macOS tray backend built on `tray-icon` (with its `muda` context menu).
//!
//! `tray-icon` is `!Send` and, on both platforms, must live on the thread that runs the
//! event loop. The caller (the app's main/UI thread) owns this sink; nothing here is moved
//! across threads.

use super::icon;
use super::{
    command_from_id, command_id, menu_entries, MenuEntry, TrayCommand, TraySink, TrayState,
};
use std::sync::Arc;
use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

pub fn create(
    handler: Arc<dyn Fn(TrayCommand) + Send + Sync>,
    initial: &TrayState,
    _rt: Option<tokio::runtime::Handle>,
) -> Option<Box<dyn TraySink>> {
    let image = icon::load(false)?;
    let tray_icon = tray_icon::Icon::from_rgba(image.data, image.width, image.height).ok()?;
    let tray = TrayIconBuilder::new()
        .with_icon(tray_icon)
        .with_tooltip("Flamix")
        .with_menu(Box::new(build_menu(initial)))
        .with_menu_on_left_click(false)
        .build()
        .ok()?;

    let click = Arc::clone(&handler);
    TrayIconEvent::set_event_handler(Some(move |event: TrayIconEvent| {
        if let TrayIconEvent::Click {
            button: MouseButton::Left,
            button_state: MouseButtonState::Up,
            ..
        } = event
        {
            click(TrayCommand::Show);
        }
    }));

    MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
        if let Some(command) = command_from_id(event.id().0.as_str()) {
            handler(command);
        }
    }));

    Some(Box::new(DesktopTray { tray }))
}

fn build_menu(state: &TrayState) -> Menu {
    let menu = Menu::new();
    for entry in menu_entries(state) {
        match entry {
            MenuEntry::Separator => {
                let _ = menu.append(&PredefinedMenuItem::separator());
            }
            MenuEntry::Item {
                label,
                enabled,
                checked,
                command,
            } => match (checked, command) {
                (Some(checked), Some(command)) => {
                    let item =
                        CheckMenuItem::with_id(command_id(command), &label, enabled, checked, None);
                    let _ = menu.append(&item);
                }
                (_, Some(command)) => {
                    let item = MenuItem::with_id(command_id(command), &label, enabled, None);
                    let _ = menu.append(&item);
                }
                (_, None) => {
                    let item = MenuItem::with_id(format!("info-{label}"), &label, false, None);
                    let _ = menu.append(&item);
                }
            },
        }
    }
    menu
}

struct DesktopTray {
    tray: TrayIcon,
}

impl TraySink for DesktopTray {
    fn apply(&self, state: &TrayState) {
        let loaded = icon::load(!state.connected)
            .and_then(|i| tray_icon::Icon::from_rgba(i.data, i.width, i.height).ok());
        if let Some(icon) = loaded {
            let _ = self.tray.set_icon(Some(icon));
        }
        self.tray.set_menu(Some(Box::new(build_menu(state))));
        let _ = self.tray.set_tooltip(Some(super::tooltip(state)));
    }
}

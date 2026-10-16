//! System tray integration (T18).
//!
//! A tiny platform-agnostic API on top of two backends:
//! * Windows/macOS — [`tray-icon`] (with its `muda` menu),
//! * Linux — [`ksni`] (StatusNotifierItem over D-Bus, pure Rust, no GTK).
//!
//! The platform backends only receive a [`TrayState`] snapshot; the pure menu model
//! ([`menu_entries`]) and the icon handling ([`icon`]) are shared and unit-tested. Tray
//! creation never panics: when the environment has no usable tray, [`Tray::new`] returns
//! `None` and the app keeps working without one.

pub mod icon;

#[cfg(any(target_os = "windows", target_os = "macos"))]
mod desktop;
#[cfg(any(target_os = "windows", target_os = "macos"))]
use desktop as platform;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
use linux as platform;

#[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
mod unsupported;
#[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
use unsupported as platform;

use std::cell::RefCell;
use std::sync::Arc;

/// What the tray menu currently shows.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TrayState {
    /// A core is connected (or connecting).
    pub connected: bool,
    /// Name of the selected server (empty when none is selected).
    pub server: String,
    /// True for TUN mode, false for Proxy mode.
    pub tun: bool,
}

/// A user action coming from the tray, handed back to the app.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayCommand {
    /// Connect if disconnected, disconnect if connected.
    ToggleConnect,
    /// Switch connection mode: `true` = TUN, `false` = Proxy.
    SetMode(bool),
    /// Bring the main window to the front.
    Show,
    /// Disconnect, save the state and exit.
    Quit,
}

/// One entry of the pure menu model (before it is materialised by a backend).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MenuEntry {
    Separator,
    Item {
        label: String,
        enabled: bool,
        /// `Some` renders a check/radio item; `None` a plain item.
        checked: Option<bool>,
        /// `None` is a non-interactive info item (e.g. the server name).
        command: Option<TrayCommand>,
    },
}

impl MenuEntry {
    fn item(label: impl Into<String>, command: Option<TrayCommand>) -> Self {
        Self::Item {
            label: label.into(),
            enabled: command.is_some(),
            checked: None,
            command,
        }
    }

    fn check(label: impl Into<String>, checked: bool, command: TrayCommand) -> Self {
        Self::Item {
            label: label.into(),
            enabled: true,
            checked: Some(checked),
            command: Some(command),
        }
    }
}

/// Build the menu model for a state. Pure — this is what the unit tests exercise.
///
/// Layout: connect toggle / current server / mode radio / show / quit.
pub fn menu_entries(state: &TrayState) -> Vec<MenuEntry> {
    let server = if state.server.is_empty() {
        "Сервер не выбран".to_string()
    } else {
        state.server.clone()
    };
    vec![
        MenuEntry::item(
            if state.connected {
                "Отключиться"
            } else {
                "Подключиться"
            },
            Some(TrayCommand::ToggleConnect),
        ),
        MenuEntry::item(server, None),
        MenuEntry::Separator,
        MenuEntry::item("Режим", None),
        MenuEntry::check("Proxy", !state.tun, TrayCommand::SetMode(false)),
        MenuEntry::check("TUN", state.tun, TrayCommand::SetMode(true)),
        MenuEntry::Separator,
        MenuEntry::item("Показать Flamix", Some(TrayCommand::Show)),
        MenuEntry::item("Выход", Some(TrayCommand::Quit)),
    ]
}

/// Tooltip (and, on Linux, the SNI title) for a state.
pub fn tooltip(state: &TrayState) -> String {
    if state.server.is_empty() {
        "Flamix".to_string()
    } else {
        format!("Flamix — {}", state.server)
    }
}

/// Stable id of a command, used by the `muda`/`tray-icon` menu backend.
#[cfg_attr(not(any(target_os = "windows", target_os = "macos")), allow(dead_code))]
pub fn command_id(command: TrayCommand) -> &'static str {
    match command {
        TrayCommand::ToggleConnect => "toggle",
        TrayCommand::SetMode(false) => "mode-proxy",
        TrayCommand::SetMode(true) => "mode-tun",
        TrayCommand::Show => "show",
        TrayCommand::Quit => "quit",
    }
}

/// Inverse of [`command_id`] (unknown ids are ignored).
#[cfg_attr(not(any(target_os = "windows", target_os = "macos")), allow(dead_code))]
pub fn command_from_id(id: &str) -> Option<TrayCommand> {
    match id {
        "toggle" => Some(TrayCommand::ToggleConnect),
        "mode-proxy" => Some(TrayCommand::SetMode(false)),
        "mode-tun" => Some(TrayCommand::SetMode(true)),
        "show" => Some(TrayCommand::Show),
        "quit" => Some(TrayCommand::Quit),
        _ => None,
    }
}

/// A materialised tray backend that can be told to repaint from a state snapshot.
///
/// Implementations are `!Send` on Windows/macOS (the tray lives on the UI thread), so the
/// caller must only touch them from the thread that created them.
pub trait TraySink {
    fn apply(&self, state: &TrayState);
}

/// Inputs required to create a tray.
pub struct TrayHandle {
    /// Called from the tray's own thread/loop for every menu action. The handler is
    /// responsible for marshalling onto the app (see [`TrayCommand`] users).
    pub on_command: Box<dyn Fn(TrayCommand) + Send + Sync>,
    /// Snapshot to render immediately.
    pub initial: TrayState,
    /// Tokio runtime used by the Linux (D-Bus) backend. `None` disables the tray there.
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    pub rt: Option<tokio::runtime::Handle>,
}

/// A live tray. Dropping it removes the icon. `!Send` by design (UI-thread bound).
pub struct Tray {
    sink: Box<dyn TraySink>,
    state: RefCell<TrayState>,
}

impl Tray {
    /// Create the tray for the current platform. Returns `None` when no tray is available
    /// (headless host, missing D-Bus session, unsupported platform) — never panics.
    pub fn new(handle: TrayHandle) -> Option<Tray> {
        let handler: Arc<dyn Fn(TrayCommand) + Send + Sync> = Arc::from(handle.on_command);
        let sink = platform::create(handler, &handle.initial, handle.rt)?;
        let tray = Tray {
            sink,
            state: RefCell::new(handle.initial),
        };
        // Make sure the backend starts in sync with the initial snapshot.
        let initial = tray.state.borrow().clone();
        tray.sink.apply(&initial);
        Some(tray)
    }

    /// Update the connection indicator and the current server name.
    pub fn set_connected(&self, connected: bool, server: &str) {
        self.update(|s| {
            s.connected = connected;
            s.server = server.to_string();
        });
    }

    /// Update the Proxy/TUN mode shown in the menu.
    pub fn set_mode(&self, tun: bool) {
        self.update(|s| s.tun = tun);
    }

    fn update(&self, mutate: impl FnOnce(&mut TrayState)) {
        let mut next = self.state.borrow().clone();
        mutate(&mut next);
        if *self.state.borrow() == next {
            return;
        }
        *self.state.borrow_mut() = next.clone();
        self.sink.apply(&next);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn states() -> Vec<TrayState> {
        vec![
            TrayState::default(),
            TrayState {
                connected: true,
                server: "Amsterdam".into(),
                tun: false,
            },
            TrayState {
                connected: false,
                server: "Helsinki".into(),
                tun: true,
            },
        ]
    }

    fn labels(entries: &[MenuEntry]) -> Vec<String> {
        entries
            .iter()
            .filter_map(|e| match e {
                MenuEntry::Item { label, .. } => Some(label.clone()),
                MenuEntry::Separator => None,
            })
            .collect()
    }

    #[test]
    fn menu_has_expected_items_in_order() {
        let entries = menu_entries(&states()[0]);
        assert_eq!(
            labels(&entries),
            vec![
                "Подключиться",
                "Сервер не выбран",
                "Режим",
                "Proxy",
                "TUN",
                "Показать Flamix",
                "Выход",
            ]
        );
    }

    #[test]
    fn toggle_label_follows_connection_state() {
        let connected = menu_entries(&states()[1]);
        assert_eq!(labels(&connected)[0], "Отключиться");
        assert_eq!(
            connected[0],
            MenuEntry::item("Отключиться", Some(TrayCommand::ToggleConnect))
        );
    }

    #[test]
    fn server_name_is_shown_disabled() {
        let entries = menu_entries(&states()[1]);
        assert_eq!(entries[1], MenuEntry::item("Amsterdam", None));
    }

    #[test]
    fn mode_radio_reflects_tun_flag() {
        let proxy = menu_entries(&states()[0]);
        assert_eq!(
            proxy[4],
            MenuEntry::check("Proxy", true, TrayCommand::SetMode(false))
        );
        assert_eq!(
            proxy[5],
            MenuEntry::check("TUN", false, TrayCommand::SetMode(true))
        );

        let tun = menu_entries(&states()[2]);
        assert_eq!(
            tun[4],
            MenuEntry::check("Proxy", false, TrayCommand::SetMode(false))
        );
        assert_eq!(
            tun[5],
            MenuEntry::check("TUN", true, TrayCommand::SetMode(true))
        );
    }

    #[test]
    fn every_command_id_round_trips() {
        for command in [
            TrayCommand::ToggleConnect,
            TrayCommand::SetMode(false),
            TrayCommand::SetMode(true),
            TrayCommand::Show,
            TrayCommand::Quit,
        ] {
            assert_eq!(command_from_id(command_id(command)), Some(command));
        }
        assert_eq!(command_from_id("nope"), None);
    }

    #[test]
    fn tooltip_falls_back_to_app_name_without_server() {
        assert_eq!(tooltip(&TrayState::default()), "Flamix");
        let state = TrayState {
            server: "Amsterdam".into(),
            ..TrayState::default()
        };
        assert_eq!(tooltip(&state), "Flamix — Amsterdam");
    }
}

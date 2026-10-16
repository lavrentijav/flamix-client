//! Portable export / import of the whole [`AppState`] (task T20).
//!
//! The snapshot is a self-describing JSON wrapper: [`FORMAT`] identifies Flamix files and
//! [`VERSION`] lets a newer app reject nothing while an older app rejects a newer copy cleanly.
//! The machine-local `device_id` (sent as `x-hwid`) is never written to or read from a file,
//! so a backup can be shared between machines without leaking hardware ids.

use crate::error::{Error, Result};
use crate::model::AppState;
use serde::{Deserialize, Serialize};

/// Marker of every Flamix backup file (a foreign JSON must not be silently accepted).
pub const FORMAT: &str = "flamix-backup";

/// Backup format version; bump only on an incompatible change.
pub const VERSION: u32 = 1;

/// The whole state wrapped with its format metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Backup {
    /// Must equal [`FORMAT`].
    pub format: String,
    /// Must be `<= VERSION`.
    pub version: u32,
    /// `CARGO_PKG_VERSION` of the writer.
    pub app_version: String,
    /// Creation time, unix seconds.
    pub exported_at: i64,
    pub state: AppState,
}

/// Serialise `state` to pretty JSON, stripping the machine-local `device_id`.
pub fn export(state: &AppState) -> Result<String> {
    let mut state = state.clone();
    state.device_id = None;
    let backup = Backup {
        format: FORMAT.to_string(),
        version: VERSION,
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        exported_at: chrono::Utc::now().timestamp(),
        state,
    };
    Ok(serde_json::to_string_pretty(&backup)?)
}

/// Parse a backup file and return the contained state (with `device_id` removed).
///
/// Rejects files that are not Flamix backups and those written by a newer format version.
pub fn import(text: &str) -> Result<AppState> {
    let mut backup: Backup = serde_json::from_str(text)?;
    if backup.format != FORMAT {
        return Err(Error::Parse("это не резервная копия Flamix".to_string()));
    }
    if backup.version > VERSION {
        return Err(Error::Parse(
            "копия новее этой версии приложения".to_string(),
        ));
    }
    backup.state.device_id = None;
    Ok(backup.state)
}

/// Overwrite every field of `current` with `imported`, keeping this machine's own `device_id`
/// and forcing the current state schema version.
pub fn apply_imported(current: &mut AppState, imported: AppState) {
    let device_id = current.device_id.take();
    *current = imported;
    current.device_id = device_id;
    current.version = AppState::VERSION;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Credentials, Security, Server, Settings, Subscription, TrafficInfo};
    use std::collections::BTreeMap;

    fn server(id: &str) -> Server {
        Server {
            id: id.to_string(),
            name: format!("Server {id}"),
            country: Some("DE".to_string()),
            address: "example.com".to_string(),
            port: 443,
            credentials: Credentials::Trojan {
                password: "secret".to_string(),
            },
            transport: Default::default(),
            security: Security::None,
            raw_link: Some(format!("trojan://secret@example.com:443#{id}")),
            traffic: None,
            local_traffic: TrafficInfo {
                upload: 10,
                download: 20,
                total: None,
                expire: None,
            },
            last_ping_ms: Some(Some(42)),
        }
    }

    fn subscription(id: &str) -> Subscription {
        Subscription {
            id: id.to_string(),
            name: format!("Sub {id}"),
            auto_title: None,
            url: format!("https://example.com/{id}"),
            backup_urls: vec!["https://backup.example.com/sub".to_string()],
            last_working_url: None,
            update_interval_hours: 12,
            last_update: Some(1_700_000_000),
            last_error: None,
            traffic: None,
            support_url: None,
            web_page_url: None,
            announce: Some("hello".to_string()),
            user_agent: None,
            pinned: true,
            collapsed: false,
            servers: vec![server(&format!("{id}-srv"))],
            provider: Default::default(),
            options: Default::default(),
        }
    }

    fn sample_state() -> AppState {
        let settings = Settings {
            socks_port: 12345,
            mux: true,
            ..Settings::default()
        };

        let mut daily_traffic = BTreeMap::new();
        daily_traffic.insert(
            "2026-01-01".to_string(),
            TrafficInfo {
                upload: 1,
                download: 2,
                total: Some(100),
                expire: None,
            },
        );

        AppState {
            version: AppState::VERSION,
            subscriptions: vec![subscription("sub-1")],
            local_servers: vec![server("local-1")],
            selected_server: Some("sub-1-srv".to_string()),
            settings,
            daily_traffic,
            device_id: None,
        }
    }

    fn backup_of(state: &AppState) -> Backup {
        Backup {
            format: FORMAT.to_string(),
            version: VERSION,
            app_version: "test".to_string(),
            exported_at: 0,
            state: state.clone(),
        }
    }

    #[test]
    fn export_import_roundtrip() {
        let state = sample_state();
        let text = export(&state).unwrap();
        let back = import(&text).unwrap();
        assert_eq!(back, state);
    }

    #[test]
    fn export_strips_device_id_and_import_ignores_it() {
        let mut state = sample_state();
        state.device_id = Some("deadbeef".to_string());
        let text = export(&state).unwrap();

        let parsed: Backup = serde_json::from_str(&text).unwrap();
        assert!(parsed.state.device_id.is_none());

        let back = import(&text).unwrap();
        assert!(back.device_id.is_none());
    }

    #[test]
    fn foreign_format_is_rejected() {
        let mut backup = backup_of(&sample_state());
        backup.format = "not-a-flamix-backup".to_string();
        let text = serde_json::to_string(&backup).unwrap();
        match import(&text) {
            Err(Error::Parse(msg)) => assert_eq!(msg, "это не резервная копия Flamix"),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn newer_version_is_rejected() {
        let mut backup = backup_of(&sample_state());
        backup.version = VERSION + 1;
        let text = serde_json::to_string(&backup).unwrap();
        match import(&text) {
            Err(Error::Parse(msg)) => assert_eq!(msg, "копия новее этой версии приложения"),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn apply_imported_keeps_local_device_id() {
        let mut current = AppState {
            device_id: Some("mine".to_string()),
            ..AppState::default()
        };
        let mut imported = sample_state();
        imported.device_id = Some("foreign".to_string());
        imported.version = 0;

        apply_imported(&mut current, imported.clone());

        assert_eq!(current.device_id.as_deref(), Some("mine"));
        assert_eq!(current.subscriptions, imported.subscriptions);
        assert_eq!(current.local_servers, imported.local_servers);
        assert_eq!(current.settings, imported.settings);
        assert_eq!(current.version, AppState::VERSION);
    }
}

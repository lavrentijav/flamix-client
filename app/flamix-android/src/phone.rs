//! Phone screen logic (M4): server list, selection, adding subscriptions and building the VPN
//! config. Everything here is plain Rust (no Slint, no JNI) so it is unit-tested on the host.
//!
//! The state file is the same `state.json` the desktop app uses (`flamix_core::store`). Every
//! change is load -> mutate -> save on the UI thread, so the background subscription fetch never
//! touches the file itself.

use flamix_core::config::{self, android_tun, ConnectContext, CoreKind};
use flamix_core::model::{AppState, Server, Subscription};
use flamix_core::{store, subscription};
use std::path::Path;

/// Shown when the selected server has no sing-box outbound (e.g. xhttp, which needs Xray).
pub const UNSUPPORTED_MESSAGE: &str = "Этот сервер не поддерживается на Android";

/// One row of the server list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerRow {
    pub id: String,
    pub name: String,
    pub subtitle: String,
}

/// Every server the phone list shows, in order: subscriptions first, then local servers.
/// The list index used by the UI is the index into this iterator.
pub fn all_servers(state: &AppState) -> impl Iterator<Item = &Server> {
    state
        .subscriptions
        .iter()
        .flat_map(|sub| sub.servers.iter())
        .chain(state.local_servers.iter())
}

pub fn server_rows(state: &AppState) -> Vec<ServerRow> {
    all_servers(state)
        .map(|s| ServerRow {
            id: s.id.clone(),
            name: s.name.clone(),
            subtitle: format!(
                "{} · {}",
                s.protocol().as_str(),
                s.transport.network.as_str()
            ),
        })
        .collect()
}

/// Id of the server at list position `index`.
pub fn server_id_at(state: &AppState, index: usize) -> Option<String> {
    all_servers(state).nth(index).map(|s| s.id.clone())
}

pub fn selected_server(state: &AppState) -> Option<&Server> {
    let id = state.selected_server.as_deref()?;
    all_servers(state).find(|s| s.id == id)
}

/// Name shown under the connect button.
pub fn selected_name(state: &AppState) -> String {
    selected_server(state)
        .map(|s| s.name.clone())
        .unwrap_or_else(|| "Сервер не выбран".into())
}

/// Builds the sing-box config (JSON text) for the selected server, the way the desktop
/// controller does for a single server: the owning subscription's provider routing applies
/// unless the subscription ignores it.
pub fn vpn_config_json(state: &AppState) -> Result<String, String> {
    let id = state
        .selected_server
        .as_deref()
        .ok_or_else(|| "Сначала выберите сервер".to_string())?;
    let (server, owner) = state
        .subscriptions
        .iter()
        .find_map(|sub| {
            sub.servers
                .iter()
                .find(|s| s.id == id)
                .map(|s| (s, Some(sub)))
        })
        .or_else(|| {
            state
                .local_servers
                .iter()
                .find(|s| s.id == id)
                .map(|s| (s, None))
        })
        .ok_or_else(|| "Выбранный сервер не найден".to_string())?;

    let provider = owner
        .filter(|sub| !sub.options.ignore_provider_routing)
        .and_then(|sub| sub.provider.routing.as_ref());
    let ctx = ConnectContext {
        provider,
        overrides: None,
    };
    let api_port = config::api_port(CoreKind::SingBox, &state.settings);
    let cfg = android_tun::android_config(server, &state.settings, api_port, &ctx)
        .ok_or_else(|| UNSUPPORTED_MESSAGE.to_string())?;
    serde_json::to_string(&cfg).map_err(|e| e.to_string())
}

/// A new, not yet fetched subscription for `url`. Only http(s) URLs are accepted.
pub fn new_subscription(url: &str) -> Result<Subscription, String> {
    let url = url.trim();
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err("Введите ссылку, начинающуюся с https://".into());
    }
    Ok(Subscription {
        id: random_hex(8),
        name: String::new(),
        auto_title: None,
        url: url.to_string(),
        backup_urls: Vec::new(),
        last_working_url: None,
        update_interval_hours: 0,
        last_update: None,
        last_error: None,
        traffic: None,
        support_url: None,
        web_page_url: None,
        announce: None,
        user_agent: None,
        pinned: false,
        collapsed: false,
        servers: Vec::new(),
        provider: Default::default(),
        options: Default::default(),
    })
}

/// Downloads `sub` on the calling thread (blocking; run it off the UI thread).
/// Returns the fetch error text, if any; `sub` is updated in place either way.
pub fn fetch_subscription(sub: &mut Subscription, hwid: Option<String>) -> Result<(), String> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("runtime: {e}"))?;
    let opts = subscription::FetchOptions {
        proxy: None,
        hwid,
        timeout: None,
    };
    rt.block_on(subscription::update(sub, &opts))
        .map_err(|e| e.to_string())
}

/// Adds `sub` to the state file at `dir`. Returns the new state, or an error text when the same
/// URL is already present.
pub fn add_subscription(dir: &Path, sub: Subscription) -> Result<AppState, String> {
    let mut state = store::load(dir).map_err(|e| e.to_string())?;
    if state.subscriptions.iter().any(|s| s.url == sub.url) {
        return Err("Эта подписка уже добавлена".into());
    }
    state.subscriptions.push(sub);
    store::save(dir, &state).map_err(|e| e.to_string())?;
    Ok(state)
}

/// Loads the state file, mutates it and saves it back. Returns the saved state.
pub fn update_state(dir: &Path, f: impl FnOnce(&mut AppState)) -> Result<AppState, String> {
    let mut state = store::load(dir).map_err(|e| e.to_string())?;
    f(&mut state);
    store::save(dir, &state).map_err(|e| e.to_string())?;
    Ok(state)
}

/// Random lowercase hex id (same scheme as the desktop controller).
fn random_hex(bytes: usize) -> String {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};
    use std::time::{SystemTime, UNIX_EPOCH};

    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let mut seed = RandomState::new().build_hasher();
    seed.write_u128(nanos);
    seed.write_u32(std::process::id());
    let mut out = String::with_capacity(bytes * 2);
    let mut value = seed.finish();
    while out.len() < bytes * 2 {
        for byte in value.to_le_bytes() {
            out.push_str(&format!("{byte:02x}"));
            if out.len() >= bytes * 2 {
                break;
            }
        }
        value = value.rotate_left(13).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_non_http_urls() {
        assert!(new_subscription("ftp://x").is_err());
        assert!(new_subscription("   ").is_err());
        let sub = new_subscription("  https://example.com/sub  ").unwrap();
        assert_eq!(sub.url, "https://example.com/sub");
        assert_eq!(sub.id.len(), 16);
    }

    #[test]
    fn random_hex_has_requested_length() {
        assert_eq!(random_hex(8).len(), 16);
        assert!(random_hex(8).chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn empty_state_has_no_rows_and_no_connect_target() {
        let state = AppState::default();
        assert!(server_rows(&state).is_empty());
        assert_eq!(selected_name(&state), "Сервер не выбран");
        assert!(vpn_config_json(&state).is_err());
    }
}

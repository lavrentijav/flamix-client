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
    /// Protocol / transport summary.
    pub subtitle: String,
    /// Address shown on the row: the measured exit IP when known, else the entry address.
    pub address: String,
    /// True when `address` is the measured exit IP (false = entry address, shown as "вход").
    pub is_exit: bool,
    /// Number of servers (>1) sharing this exit IP; 0 = unique or unknown exit.
    pub group: usize,
}

/// Servers in storage order: subscriptions first, then local servers (unordered by exit).
fn stored_servers(state: &AppState) -> Vec<&Server> {
    state
        .subscriptions
        .iter()
        .flat_map(|sub| sub.servers.iter())
        .chain(state.local_servers.iter())
        .collect()
}

/// Measured exit IP of `id`, `None` when never probed (an entry address is never a substitute).
fn exit_ip<'a>(state: &'a AppState, id: &str) -> Option<&'a str> {
    state
        .exit_ips
        .get(id)
        .map(|e| e.ip.as_str())
        .filter(|ip| !ip.is_empty())
}

/// THE grouping function. Groups server ids by their measured exit IP (`exit_ips`), never by the
/// entry address/domain. Servers with an unknown exit IP are not in any group: each one stands
/// alone. Groups are in order of their first member's position; ids inside keep list order.
pub fn exit_groups(state: &AppState) -> Vec<Vec<String>> {
    let mut groups: Vec<(Option<&str>, Vec<String>)> = Vec::new();
    for server in stored_servers(state) {
        let ip = exit_ip(state, &server.id);
        match ip.and_then(|ip| groups.iter_mut().find(|(g, _)| *g == Some(ip))) {
            Some((_, ids)) => ids.push(server.id.clone()),
            None => groups.push((ip, vec![server.id.clone()])),
        }
    }
    groups.into_iter().map(|(_, ids)| ids).collect()
}

/// Every server the phone list shows, in list order: members of one exit group are adjacent.
/// The list index used by the UI is the index into this vector.
pub fn all_servers(state: &AppState) -> Vec<&Server> {
    let all = stored_servers(state);
    exit_groups(state)
        .into_iter()
        .flatten()
        .filter_map(|id| all.iter().copied().find(|s| s.id == id))
        .collect()
}

pub fn server_rows(state: &AppState) -> Vec<ServerRow> {
    let groups = exit_groups(state);
    all_servers(state)
        .into_iter()
        .map(|s| {
            let exit = exit_ip(state, &s.id);
            let group = exit
                .and_then(|_| groups.iter().find(|g| g.contains(&s.id)))
                .map_or(0, |g| g.len())
                .max(1);
            ServerRow {
                id: s.id.clone(),
                name: s.name.clone(),
                subtitle: format!(
                    "{} · {}",
                    s.protocol().as_str(),
                    s.transport.network.as_str()
                ),
                address: exit.map_or_else(|| s.address.clone(), str::to_string),
                is_exit: exit.is_some(),
                group: if group > 1 { group } else { 0 },
            }
        })
        .collect()
}

/// Backups of `id`: the explicit `settings.failover.backups` when set, otherwise the other
/// servers with the same measured exit IP. Never chosen by address; a server with an unknown
/// exit IP has no implicit backups and is nobody's backup.
pub fn backup_ids(state: &AppState, id: &str) -> Vec<String> {
    let explicit: Vec<String> = state
        .settings
        .failover
        .backups
        .iter()
        .filter(|b| b.as_str() != id)
        .cloned()
        .collect();
    if !explicit.is_empty() {
        return explicit;
    }
    exit_groups(state)
        .into_iter()
        .find(|g| g.iter().any(|x| x == id))
        .filter(|_| exit_ip(state, id).is_some())
        .map(|g| g.into_iter().filter(|x| x != id).collect())
        .unwrap_or_default()
}

/// Id of the server at list position `index`.
pub fn server_id_at(state: &AppState, index: usize) -> Option<String> {
    all_servers(state).get(index).map(|s| s.id.clone())
}

pub fn selected_server(state: &AppState) -> Option<&Server> {
    let id = state.selected_server.as_deref()?;
    all_servers(state).into_iter().find(|s| s.id == id)
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
    let backups = backup_ids(state, id);
    let backup_servers: Vec<&Server> = backups
        .iter()
        .filter_map(|b| all_servers(state).into_iter().find(|s| &s.id == b))
        .collect();
    let cfg = android_tun::android_config(server, &backup_servers, &state.settings, api_port, &ctx)
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

    fn srv(id: &str, address: &str) -> Server {
        Server {
            id: id.into(),
            name: id.into(),
            country: None,
            address: address.into(),
            port: 443,
            credentials: flamix_core::model::Credentials::Vless {
                uuid: "11111111-1111-1111-1111-111111111111".into(),
                flow: None,
                encryption: None,
            },
            transport: Default::default(),
            security: Default::default(),
            raw_link: None,
            traffic: None,
            local_traffic: Default::default(),
            last_ping_ms: None,
        }
    }

    fn exit(state: &mut AppState, id: &str, ip: &str) {
        state.exit_ips.insert(
            id.into(),
            flamix_core::model::ExitIp {
                ip: ip.into(),
                checked: 1,
            },
        );
    }

    /// a, c share an exit; b has the same ENTRY address as a but a different exit; d unknown.
    fn grouped_state() -> AppState {
        let mut st = AppState {
            local_servers: vec![
                srv("a", "same.example"),
                srv("b", "same.example"),
                srv("c", "other.example"),
                srv("d", "same.example"),
            ],
            ..Default::default()
        };
        exit(&mut st, "a", "9.9.9.9");
        exit(&mut st, "b", "8.8.8.8");
        exit(&mut st, "c", "9.9.9.9");
        st
    }

    #[test]
    fn groups_follow_exit_ip_not_entry_address() {
        let st = grouped_state();
        let groups = exit_groups(&st);
        assert_eq!(
            groups,
            vec![vec!["a", "c"], vec!["b"], vec!["d"]]
                .into_iter()
                .map(|g| g.into_iter().map(String::from).collect::<Vec<_>>())
                .collect::<Vec<_>>()
        );
        let rows = server_rows(&st);
        let ids: Vec<&str> = rows.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, ["a", "c", "b", "d"]);
        assert_eq!(rows[0].group, 2);
        assert_eq!(rows[0].address, "9.9.9.9");
        assert!(rows[0].is_exit);
        assert_eq!(rows[2].group, 0);
        // unknown exit: entry address, flagged as such
        assert_eq!(rows[3].address, "same.example");
        assert!(!rows[3].is_exit);
        assert_eq!(server_id_at(&st, 1).as_deref(), Some("c"));
    }

    #[test]
    fn backups_are_same_exit_only() {
        let mut st = grouped_state();
        assert_eq!(backup_ids(&st, "a"), ["c"]);
        assert_eq!(backup_ids(&st, "c"), ["a"]);
        assert!(
            backup_ids(&st, "b").is_empty(),
            "same entry address is not a backup"
        );
        assert!(
            backup_ids(&st, "d").is_empty(),
            "unknown exit has no backups"
        );
        st.settings.failover.backups = vec!["b".into()];
        assert_eq!(backup_ids(&st, "a"), ["b"], "explicit list wins");
    }
}

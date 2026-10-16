//! Pure mapping from [`AppState`] to the Slint models shown by `AppWindow`.
//!
//! Everything here is side-effect free so it can be unit-tested: the controller owns the
//! mutable state and calls these functions to refresh the UI. Slint `Image` values are only
//! produced in [`server_item`]; the filter/sort/format helpers stay platform independent.

use crate::flags;
use crate::{ServerItem, SubscriptionItem};
use flamix_core::model::{AppState, Server, Subscription, TrafficInfo};
use slint::{ModelRc, VecModel};
use std::collections::{BTreeMap, HashSet};

/// `ServerItem.ping`: not tested yet.
pub const PING_UNTESTED: i32 = -2;
/// `ServerItem.ping`: test failed.
pub const PING_FAILED: i32 = -1;

/// Human-readable binary size (`15 GB`, `1.2 MB`). 1024 base, one decimal below 10.
pub fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 6] = ["B", "KB", "MB", "GB", "TB", "PB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if value < 10.0 {
        format!("{value:.1} {}", UNITS[unit])
    } else {
        format!("{value:.0} {}", UNITS[unit])
    }
}

/// "used / total" or "∞" when the quota is unlimited.
pub fn traffic_text(info: &TrafficInfo) -> String {
    match info.total {
        Some(total) if total > 0 => {
            format!("{} / {}", human_bytes(info.used()), human_bytes(total))
        }
        _ => "∞".to_string(),
    }
}

/// Quota fraction 0.0..=1.0 (`0.0` when unlimited).
pub fn traffic_fraction(info: &TrafficInfo) -> f32 {
    info.used_fraction().unwrap_or(0.0)
}

/// "до dd.mm.yyyy" for the expiry timestamp, empty when it never expires.
pub fn expire_text(info: &TrafficInfo) -> String {
    let Some(seconds) = info.expire.filter(|s| *s > 0) else {
        return String::new();
    };
    match chrono::DateTime::from_timestamp(seconds, 0) {
        Some(dt) => {
            let local = dt.with_timezone(&chrono::Local);
            format!("до {}", local.format("%d.%m.%Y"))
        }
        None => String::new(),
    }
}

/// Per-server quota: (has_quota, fraction, "осталось X").
pub fn server_quota(server: &Server) -> (bool, f32, String) {
    let Some(info) = server.traffic else {
        return (false, 0.0, String::new());
    };
    match info.remaining() {
        Some(remaining) => (
            true,
            info.used_fraction().unwrap_or(0.0),
            format!("осталось {}", human_bytes(remaining)),
        ),
        None => (false, 0.0, String::new()),
    }
}

/// Encode `Server::last_ping_ms` for the UI (`-2` untested, `-1` failed, else ms).
pub fn ping_code(server: &Server) -> i32 {
    match server.last_ping_ms {
        None => PING_UNTESTED,
        Some(None) => PING_FAILED,
        Some(Some(ms)) => ms.min(i32::MAX as u32) as i32,
    }
}

/// ISO code used for a server flag (upper case, `UN` when unknown).
pub fn server_iso(server: &Server) -> String {
    server
        .country
        .as_deref()
        .map(str::trim)
        .filter(|c| !c.is_empty())
        .map(str::to_ascii_uppercase)
        .unwrap_or_else(|| "UN".to_string())
}

/// Exit-IP display data of one server: the measured egress IP and the size of its group.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExitInfo {
    /// Measured public exit IP, empty when never probed.
    pub ip: String,
    /// 0 = no measured exit, otherwise the number of servers sharing that exit.
    pub group: i32,
}

/// Group servers by their **measured** exit IP: `ip -> sorted server ids`.
///
/// Only servers with a successful probe (`exit_ips[id].ip` non-empty) are included, so an
/// input address shared by two servers is irrelevant — the egress must have been observed.
pub fn exit_groups(state: &AppState) -> BTreeMap<String, Vec<String>> {
    let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for server in state.servers() {
        let Some(exit) = state.exit_ips.get(&server.id) else {
            continue;
        };
        if exit.ip.is_empty() {
            continue;
        }
        out.entry(exit.ip.clone())
            .or_default()
            .push(server.id.clone());
    }
    for ids in out.values_mut() {
        ids.sort();
    }
    out
}

/// Exit-IP display data for one server, resolved through the precomputed group map.
pub fn exit_info(state: &AppState, groups: &BTreeMap<String, Vec<String>>, id: &str) -> ExitInfo {
    let Some(exit) = state.exit_ips.get(id) else {
        return ExitInfo::default();
    };
    if exit.ip.is_empty() {
        return ExitInfo::default();
    }
    // A server with no group entry (should not happen) still shows its IP, group 0.
    let group = groups.get(&exit.ip).map_or(0, |ids| ids.len() as i32);
    ExitInfo {
        ip: exit.ip.clone(),
        group,
    }
}

/// Convert one server to the UI model, including the measured exit IP and its group size.
pub fn server_item_with_exit(server: &Server, selected: bool, exit: ExitInfo) -> ServerItem {
    let (has_quota, quota_fraction, quota_text) = server_quota(server);
    ServerItem {
        id: server.id.clone().into(),
        name: server.name.clone().into(),
        country: server_iso(server).into(),
        flag: flags::flag(&server_iso(server)),
        summary: server.summary().into(),
        ping: ping_code(server),
        has_quota,
        quota_fraction,
        quota_text: quota_text.into(),
        selected,
        exit_ip: exit.ip.into(),
        exit_group: exit.group,
    }
}

/// Case-insensitive substring match used by the search box.
fn matches(query_lower: &str, text: &str) -> bool {
    query_lower.is_empty() || text.to_lowercase().contains(query_lower)
}

/// Servers of a subscription visible under `query`, sorted by ping when requested.
fn filter_servers<'a>(
    servers: &'a [Server],
    query_lower: &str,
    sort_by_ping: bool,
) -> Vec<&'a Server> {
    let mut out: Vec<&Server> = servers
        .iter()
        .filter(|s| matches(query_lower, &s.name))
        .collect();
    if sort_by_ping {
        sort_servers(&mut out);
    }
    out
}

fn sort_servers(servers: &mut [&Server]) {
    servers.sort_by_key(|s| match ping_code(s) {
        code if code < 0 => i32::MAX,
        code => code,
    });
}

fn subscription_announce(sub: &Subscription) -> String {
    if let Some(banner) = &sub.provider.info_banner {
        if !banner.text.trim().is_empty() {
            return banner.text.clone();
        }
    }
    sub.announce.clone().unwrap_or_default()
}

fn subscription_error(sub: &Subscription) -> String {
    if sub
        .provider
        .hwid
        .map(|h| h.max_devices_reached)
        .unwrap_or(false)
    {
        return "Достигнут лимит устройств".to_string();
    }
    sub.last_error.clone().unwrap_or_default()
}

/// Build the subscription cards for the server list.
///
/// `updating` holds the ids of subscriptions with an update in flight; `sort_by_ping` also
/// re-orders servers inside every card (untested/failed last).
pub fn build_subscriptions(
    state: &AppState,
    query: &str,
    sort_by_ping: bool,
    updating: &HashSet<String>,
) -> Vec<SubscriptionItem> {
    let query_lower = query.to_lowercase();

    let mut subs: Vec<&Subscription> = state
        .subscriptions
        .iter()
        .filter(|sub| {
            matches(&query_lower, &sub.name)
                || sub.servers.iter().any(|s| matches(&query_lower, &s.name))
        })
        .collect();
    // Pinned subscriptions first, otherwise keep the stored order.
    subs.sort_by_key(|sub| !sub.pinned);

    let groups = exit_groups(state);
    subs.into_iter()
        .map(|sub| {
            let servers: Vec<ServerItem> = filter_servers(&sub.servers, &query_lower, sort_by_ping)
                .into_iter()
                .map(|s| {
                    let exit = exit_info(state, &groups, &s.id);
                    server_item_with_exit(s, state.selected_server.as_deref() == Some(&s.id), exit)
                })
                .collect();
            let traffic = sub.traffic;
            SubscriptionItem {
                id: sub.id.clone().into(),
                name: sub.name.clone().into(),
                pinned: sub.pinned,
                collapsed: sub.collapsed,
                updating: updating.contains(&sub.id),
                has_traffic: traffic.is_some(),
                traffic_fraction: traffic.map(|t| traffic_fraction(&t)).unwrap_or(0.0),
                traffic_text: traffic
                    .map(|t| traffic_text(&t))
                    .unwrap_or_else(|| "∞".to_string())
                    .into(),
                expire_text: traffic.map(|t| expire_text(&t)).unwrap_or_default().into(),
                announce: subscription_announce(sub).into(),
                error: subscription_error(sub).into(),
                servers: model(servers),
            }
        })
        .collect()
}

/// Build the standalone (local) server list. `query` filters by name.
pub fn build_local_servers(state: &AppState, query: &str, sort_by_ping: bool) -> Vec<ServerItem> {
    let query_lower = query.to_lowercase();
    let groups = exit_groups(state);
    filter_servers(&state.local_servers, &query_lower, sort_by_ping)
        .into_iter()
        .map(|s| {
            let exit = exit_info(state, &groups, &s.id);
            server_item_with_exit(s, state.selected_server.as_deref() == Some(&s.id), exit)
        })
        .collect()
}

/// All servers visible under `query` (subscriptions first, then local), matching the same
/// rules as the server list. Used by "ping all".
pub fn visible_servers<'a>(state: &'a AppState, query: &str) -> Vec<&'a Server> {
    let query_lower = query.to_lowercase();
    let mut out: Vec<&Server> = Vec::new();
    for sub in &state.subscriptions {
        if matches(&query_lower, &sub.name) {
            out.extend(sub.servers.iter());
        } else {
            out.extend(
                sub.servers
                    .iter()
                    .filter(|s| matches(&query_lower, &s.name)),
            );
        }
    }
    out.extend(
        state
            .local_servers
            .iter()
            .filter(|s| matches(&query_lower, &s.name)),
    );
    out
}

/// Name of the selected server, empty when nothing is selected.
pub fn selected_name(state: &AppState) -> String {
    state.selected().map(|s| s.name.clone()).unwrap_or_default()
}

/// ISO code of the selected server, empty when nothing is selected.
pub fn selected_iso(state: &AppState) -> String {
    state.selected().map(server_iso).unwrap_or_default()
}

/// Wrap a vector in the Slint model type used by the UI properties.
pub fn model<T: Clone + 'static>(items: Vec<T>) -> ModelRc<T> {
    ModelRc::new(VecModel::from(items))
}

#[cfg(test)]
mod tests {
    use super::*;
    use flamix_core::model::{Credentials, ProviderExtras, Security, Subscription};
    use slint::Model;

    fn server(id: &str, name: &str, ping: Option<Option<u32>>) -> Server {
        Server {
            id: id.to_string(),
            name: name.to_string(),
            country: Some("de".to_string()),
            address: "example.com".to_string(),
            port: 443,
            credentials: Credentials::Trojan {
                password: "p".to_string(),
            },
            transport: Default::default(),
            security: Security::None,
            raw_link: None,
            traffic: None,
            local_traffic: TrafficInfo::default(),
            last_ping_ms: ping,
        }
    }

    fn subscription(id: &str, name: &str, servers: Vec<Server>) -> Subscription {
        Subscription {
            id: id.to_string(),
            name: name.to_string(),
            auto_title: None,
            url: "https://example.com/sub".to_string(),
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
            servers,
            provider: ProviderExtras::default(),
            options: Default::default(),
        }
    }

    #[test]
    fn human_bytes_binary_one_decimal_under_ten() {
        assert_eq!(human_bytes(0), "0 B");
        assert_eq!(human_bytes(512), "512 B");
        assert_eq!(human_bytes(1024), "1.0 KB");
        assert_eq!(human_bytes(1536), "1.5 KB");
        assert_eq!(human_bytes(15 * 1024 * 1024 * 1024), "15 GB");
        assert_eq!(human_bytes(100 * 1024 * 1024 * 1024), "100 GB");
        // 1.2 MB
        assert_eq!(human_bytes(12 * 1024 * 1024 / 10), "1.2 MB");
    }

    #[test]
    fn traffic_text_used_over_total_and_infinity() {
        let t = TrafficInfo {
            upload: 5,
            download: 10,
            total: Some(2 * 1024 * 1024 * 1024 * 1024),
            expire: None,
        };
        assert_eq!(traffic_text(&t), "15 B / 2.0 TB");
        let unlimited = TrafficInfo { total: None, ..t };
        assert_eq!(traffic_text(&unlimited), "∞");
    }

    #[test]
    fn expire_text_formats_date_and_skips_zero() {
        // 2027-01-01 00:00:00 UTC, rendered in the machine's local zone.
        let info = TrafficInfo {
            expire: Some(1798761600),
            ..TrafficInfo::default()
        };
        let text = expire_text(&info);
        assert!(text.starts_with("до "), "{text}");
        let none = TrafficInfo {
            expire: Some(0),
            ..TrafficInfo::default()
        };
        assert_eq!(expire_text(&none), "");
    }

    #[test]
    fn server_quota_remaining_text() {
        let mut s = server("a", "A", None);
        assert_eq!(server_quota(&s), (false, 0.0, String::new()));
        s.traffic = Some(TrafficInfo {
            upload: 0,
            download: 1024 * 1024 * 1024,
            total: Some(4 * 1024 * 1024 * 1024),
            expire: None,
        });
        let (has, fraction, text) = server_quota(&s);
        assert!(has);
        assert!((fraction - 0.25).abs() < 1e-6);
        assert_eq!(text, "осталось 3.0 GB");
    }

    #[test]
    fn ping_code_encoding() {
        assert_eq!(ping_code(&server("a", "A", None)), PING_UNTESTED);
        assert_eq!(ping_code(&server("a", "A", Some(None))), PING_FAILED);
        assert_eq!(ping_code(&server("a", "A", Some(Some(42)))), 42);
    }

    #[test]
    fn server_iso_upper_cases_and_defaults() {
        assert_eq!(server_iso(&server("a", "A", None)), "DE");
        let mut s = server("a", "A", None);
        s.country = None;
        assert_eq!(server_iso(&s), "UN");
    }

    #[test]
    fn search_filters_servers_and_hides_empty_subscriptions() {
        let state = AppState {
            subscriptions: vec![
                subscription(
                    "s1",
                    "Premium",
                    vec![server("a", "Frankfurt", None), server("b", "Berlin", None)],
                ),
                subscription("s2", "Basic", vec![server("c", "New York", None)]),
            ],
            ..AppState::default()
        };
        let items = build_subscriptions(&state, "ber", false, &HashSet::new());
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].name, "Premium");
        assert_eq!(items[0].servers.row_count(), 1);
        assert_eq!(items[0].servers.row_data(0).unwrap().name, "Berlin");

        let all = build_subscriptions(&state, "", false, &HashSet::new());
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].name, "Premium");
        assert_eq!(all[1].name, "Basic");
    }

    #[test]
    fn pinned_subscriptions_come_first() {
        let mut state = AppState::default();
        state
            .subscriptions
            .push(subscription("s1", "First", vec![server("a", "A", None)]));
        let mut pinned = subscription("s2", "Pinned", vec![server("b", "B", None)]);
        pinned.pinned = true;
        state.subscriptions.push(pinned);
        let items = build_subscriptions(&state, "", false, &HashSet::new());
        assert_eq!(items[0].name, "Pinned");
        assert!(items[0].pinned);
        assert_eq!(items[1].name, "First");
    }

    #[test]
    fn sort_by_ping_puts_failed_and_untested_last() {
        let state = AppState {
            subscriptions: vec![subscription(
                "s1",
                "S",
                vec![
                    server("a", "Slow", Some(Some(300))),
                    server("b", "Fast", Some(Some(20))),
                    server("c", "Untested", None),
                    server("d", "Failed", Some(None)),
                ],
            )],
            ..AppState::default()
        };
        let items = build_subscriptions(&state, "", true, &HashSet::new());
        let names: Vec<String> = (0..items[0].servers.row_count())
            .map(|i| items[0].servers.row_data(i).unwrap().name.to_string())
            .collect();
        assert_eq!(names, vec!["Fast", "Slow", "Untested", "Failed"]);
    }

    #[test]
    fn local_servers_filtered_by_name() {
        let state = AppState {
            local_servers: vec![server("l1", "Local One", None), server("l2", "Other", None)],
            ..AppState::default()
        };
        let items = build_local_servers(&state, "local", false);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].name, "Local One");
    }

    #[test]
    fn visible_servers_honours_query() {
        let state = AppState {
            subscriptions: vec![subscription(
                "s1",
                "Premium",
                vec![server("a", "Frankfurt", None), server("b", "Berlin", None)],
            )],
            local_servers: vec![server("l1", "Local One", None)],
            ..AppState::default()
        };
        assert_eq!(visible_servers(&state, "").len(), 3);
        let filtered = visible_servers(&state, "ber");
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].id, "b");
        // Subscription name match keeps all its servers.
        assert_eq!(visible_servers(&state, "premium").len(), 2);
    }

    #[test]
    fn banners_and_hwid_error_and_announce_field() {
        use flamix_core::model::{HwidStatus, InfoBanner};
        let mut sub = subscription("s1", "S", vec![server("a", "A", None)]);
        sub.announce = Some("plain announce".to_string());
        assert_eq!(subscription_announce(&sub), "plain announce");
        sub.provider.info_banner = Some(InfoBanner {
            text: "banner text".to_string(),
            ..InfoBanner::default()
        });
        assert_eq!(subscription_announce(&sub), "banner text");
        assert_eq!(subscription_error(&sub), "");
        sub.provider.hwid = Some(HwidStatus {
            max_devices_reached: true,
            ..HwidStatus::default()
        });
        assert_eq!(subscription_error(&sub), "Достигнут лимит устройств");
    }

    #[test]
    fn exit_groups_only_measured_and_sorted() {
        use flamix_core::model::ExitIp;
        let mut state = AppState {
            local_servers: vec![server("b", "B", None), server("a", "A", None)],
            ..AppState::default()
        };
        state.exit_ips.insert(
            "a".to_string(),
            ExitIp {
                ip: "203.0.113.7".to_string(),
                checked: 1,
            },
        );
        state.exit_ips.insert(
            "b".to_string(),
            ExitIp {
                ip: "203.0.113.7".to_string(),
                checked: 1,
            },
        );
        // Unmeasured and empty-ip entries are excluded.
        state.exit_ips.insert(
            "c".to_string(),
            ExitIp {
                ip: String::new(),
                checked: 1,
            },
        );

        let groups = exit_groups(&state);
        assert_eq!(groups.len(), 1);
        assert_eq!(
            groups.get("203.0.113.7").unwrap(),
            &vec!["a".to_string(), "b".to_string()]
        );
    }

    #[test]
    fn server_item_carries_exit_ip_and_group() {
        use flamix_core::model::ExitIp;
        let mut state = AppState {
            local_servers: vec![server("a", "A", None), server("b", "B", None)],
            ..AppState::default()
        };
        state.exit_ips.insert(
            "a".to_string(),
            ExitIp {
                ip: "203.0.113.7".to_string(),
                checked: 1,
            },
        );
        state.exit_ips.insert(
            "b".to_string(),
            ExitIp {
                ip: "203.0.113.7".to_string(),
                checked: 1,
            },
        );

        let items = build_local_servers(&state, "", false);
        for item in &items {
            assert_eq!(item.exit_ip.as_str(), "203.0.113.7");
            assert_eq!(item.exit_group, 2);
        }
    }

    #[test]
    fn server_item_without_measurement_has_empty_exit() {
        let state = AppState {
            local_servers: vec![server("a", "A", None)],
            ..AppState::default()
        };
        let items = build_local_servers(&state, "", false);
        assert_eq!(items[0].exit_ip.as_str(), "");
        assert_eq!(items[0].exit_group, 0);
    }

    #[test]
    fn selected_server_name_and_iso() {
        let mut state = AppState {
            subscriptions: vec![subscription("s1", "S", vec![server("a", "Chosen", None)])],
            ..AppState::default()
        };
        assert_eq!(selected_name(&state), "");
        state.selected_server = Some("a".to_string());
        assert_eq!(selected_name(&state), "Chosen");
        assert_eq!(selected_iso(&state), "DE");
    }
}

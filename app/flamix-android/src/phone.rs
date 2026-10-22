//! Phone screen logic (M4): server list, selection, adding subscriptions and building the VPN
//! config. Everything here is plain Rust (no Slint, no JNI) so it is unit-tested on the host.
//!
//! The state file is the same `state.json` the desktop app uses (`flamix_core::store`). Every
//! change is load -> mutate -> save on the UI thread, so the background subscription fetch never
//! touches the file itself.

use flamix_core::config::{self, android_tun, ConnectContext, CoreKind};
use flamix_core::model::{AppState, Server, Subscription};
use flamix_core::provider_routing::RoutingLinkAction;
use flamix_core::{deeplink, link, store, subscription};
use std::path::Path;

/// Shown when the selected server has no sing-box outbound (e.g. xhttp, which needs Xray).
pub const UNSUPPORTED_MESSAGE: &str = "Этот сервер не поддерживается на Android";

/// One row of the server list.
#[derive(Debug, Clone, PartialEq)]
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
    /// Desktop encoding: [`PING_UNTESTED`], [`PING_FAILED`] or the latency in ms.
    pub ping: i32,
    /// Quota fraction used and "осталось X" text, when the server reports a quota.
    pub quota: Option<(f32, String)>,
}

/// `ServerRow::ping`: not tested yet.
pub const PING_UNTESTED: i32 = -2;
/// `ServerRow::ping`: the last test failed.
pub const PING_FAILED: i32 = -1;

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
pub fn all_servers(state: &AppState) -> Vec<&Server> {
    let all = stored_servers(state);
    exit_groups(state)
        .into_iter()
        .flatten()
        .filter_map(|id| all.iter().copied().find(|s| s.id == id))
        .collect()
}

/// Row for one server; `groups` is the result of [`exit_groups`] for `state`.
fn row(state: &AppState, server: &Server, groups: &[Vec<String>]) -> ServerRow {
    let exit = exit_ip(state, &server.id);
    let group = exit
        .and_then(|_| groups.iter().find(|g| g.contains(&server.id)))
        .map_or(0, Vec::len);
    let quota = server.traffic.and_then(|t| {
        t.remaining().map(|left| {
            (
                t.used_fraction().unwrap_or(0.0),
                format!("осталось {}", human_bytes(left)),
            )
        })
    });
    ServerRow {
        id: server.id.clone(),
        name: server.name.clone(),
        subtitle: format!(
            "{} · {}",
            server.protocol().as_str(),
            server.transport.network.as_str()
        ),
        address: exit.map_or_else(|| server.address.clone(), str::to_string),
        is_exit: exit.is_some(),
        group: if group > 1 { group } else { 0 },
        ping: ping_code(server),
        quota,
    }
}

pub fn server_rows(state: &AppState) -> Vec<ServerRow> {
    let groups = exit_groups(state);
    all_servers(state)
        .into_iter()
        .map(|s| row(state, s, &groups))
        .collect()
}

/// Encodes `Server::last_ping_ms` like the desktop UI (see [`PING_UNTESTED`]).
fn ping_code(server: &Server) -> i32 {
    match server.last_ping_ms {
        None => PING_UNTESTED,
        Some(None) => PING_FAILED,
        Some(Some(ms)) => i32::try_from(ms).unwrap_or(i32::MAX),
    }
}

/// Binary size text: `15 GB`, `1.2 MB` (1024 base, one decimal below 10).
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

/// Longest URL handed to the OS opener (same limit as the desktop app).
const MAX_URL_LEN: usize = 2048;

/// Provider-supplied link that is safe to open: http(s), tg:// or mailto:, single line.
pub fn safe_url(raw: &str) -> Option<String> {
    let url = raw.trim();
    if url.is_empty() || url.len() > MAX_URL_LEN {
        return None;
    }
    if url.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return None;
    }
    let lower = url.to_ascii_lowercase();
    let ok = if let Some(rest) = lower
        .strip_prefix("https://")
        .or_else(|| lower.strip_prefix("http://"))
    {
        rest.split(['/', '?', '#'])
            .next()
            .is_some_and(|host| !host.is_empty() && !host.starts_with('@'))
    } else if let Some(rest) = lower.strip_prefix("tg://") {
        !rest.is_empty()
    } else if let Some(rest) = lower.strip_prefix("mailto:") {
        !rest.is_empty()
    } else {
        false
    };
    ok.then(|| url.to_string())
}

/// Banner tones (the same numbers the desktop `SubscriptionItem.banner-tone` uses).
pub const TONE_INFO: i32 = 0;
pub const TONE_RED: i32 = 1;
pub const TONE_GREEN: i32 = 2;
/// Custom `#hex` colour, carried in [`BannerView::color`].
pub const TONE_CUSTOM: i32 = 3;
/// Expiry notice.
pub const TONE_WARNING: i32 = 4;

/// Provider banner of a subscription, ready for the UI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BannerView {
    pub text: String,
    pub tone: i32,
    /// RGBA, only for [`TONE_CUSTOM`].
    pub color: Option<[u8; 4]>,
    /// Empty when there is no (valid) button.
    pub button_text: String,
    /// Checked with [`safe_url`]; empty when there is no button.
    pub link: String,
}

/// Maps a Happ banner colour (`red|blue|green|#RGB|#RRGGBB|#RRGGBBAA`) to a tone.
pub fn banner_tone(color: Option<&str>) -> (i32, Option<[u8; 4]>) {
    let Some(raw) = color.map(str::trim).filter(|c| !c.is_empty()) else {
        return (TONE_INFO, None);
    };
    match raw.to_ascii_lowercase().as_str() {
        "red" => return (TONE_RED, None),
        "green" => return (TONE_GREEN, None),
        "blue" => return (TONE_INFO, None),
        _ => {}
    }
    match parse_hex_color(raw) {
        Some(rgba) => (TONE_CUSTOM, Some(rgba)),
        None => (TONE_INFO, None),
    }
}

/// `#RGB`, `#RRGGBB` or `#RRGGBBAA` (the `#` is required).
fn parse_hex_color(raw: &str) -> Option<[u8; 4]> {
    let hex = raw.strip_prefix('#')?;
    if !hex.is_ascii() || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
    match hex.len() {
        3 => {
            let nibble = |i: usize| u8::from_str_radix(&hex[i..i + 1], 16).ok().map(|n| n * 17);
            Some([nibble(0)?, nibble(1)?, nibble(2)?, 255])
        }
        6 => Some([byte(0)?, byte(2)?, byte(4)?, 255]),
        8 => Some([byte(0)?, byte(2)?, byte(4)?, byte(6)?]),
        _ => None,
    }
}

/// The banner of a subscription: the expiry notice has priority over the info banner.
pub fn banner_view(sub: &Subscription) -> Option<BannerView> {
    let provider = &sub.provider;
    if let Some(text) = provider
        .expire_notice
        .as_deref()
        .map(str::trim)
        .filter(|t| !t.is_empty())
    {
        let link = provider.expire_button_link.as_deref().and_then(safe_url);
        return Some(BannerView {
            text: text.to_string(),
            tone: TONE_WARNING,
            color: None,
            button_text: link
                .as_ref()
                .map(|_| "Продлить".to_string())
                .unwrap_or_default(),
            link: link.unwrap_or_default(),
        });
    }
    let banner = provider.info_banner.as_ref()?;
    let text = banner.text.trim();
    if text.is_empty() {
        return None;
    }
    let (tone, color) = banner_tone(banner.color.as_deref());
    let link = banner.button_link.as_deref().and_then(safe_url);
    let button_text = match (&link, banner.button_text.as_deref().map(str::trim)) {
        (None, _) => String::new(),
        (Some(_), Some(t)) if !t.is_empty() => t.to_string(),
        (Some(_), _) => "Открыть".to_string(),
    };
    Some(BannerView {
        text: text.to_string(),
        tone,
        color,
        button_text,
        link: link.unwrap_or_default(),
    })
}

/// Support link kind: 0 none, 1 Telegram, 2 e-mail, 3 website (the icon the card shows).
pub const SUPPORT_NONE: i32 = 0;

/// Support link of a subscription as (url, kind). Unsafe links count as absent.
pub fn support_link(sub: &Subscription) -> (String, i32) {
    let Some(url) = sub.support_url.as_deref().and_then(safe_url) else {
        return (String::new(), SUPPORT_NONE);
    };
    let kind = match flamix_core::support::support_kind(&url) {
        flamix_core::support::SupportKind::Telegram => 1,
        flamix_core::support::SupportKind::Email => 2,
        flamix_core::support::SupportKind::Website => 3,
    };
    (url, kind)
}

/// One subscription card of the servers tab (`local` = the manually added servers).
#[derive(Debug, Clone, PartialEq)]
pub struct SubCard {
    /// Empty for the local-servers card.
    pub id: String,
    pub name: String,
    pub local: bool,
    pub collapsed: bool,
    pub pinned: bool,
    pub updating: bool,
    /// (text, quota fraction, expiry text) when the subscription reports traffic.
    pub traffic: Option<(String, f32, String)>,
    pub banner: Option<BannerView>,
    pub announce: String,
    pub error: String,
    pub support_url: String,
    pub support_kind: i32,
    pub web_url: String,
    pub servers: Vec<ServerRow>,
}

/// Subscription line: "осталось X из Y", or "без лимита" with the used amount.
fn traffic_line(info: &flamix_core::model::TrafficInfo) -> (String, f32) {
    let fraction = info.used_fraction().unwrap_or(0.0);
    let text = match (info.total, info.remaining()) {
        (Some(total), Some(left)) if total > 0 => {
            format!("осталось {} из {}", human_bytes(left), human_bytes(total))
        }
        _ => format!("без лимита, использовано {}", human_bytes(info.used())),
    };
    (text, fraction)
}

/// "до dd.mm.yyyy" for the expiry timestamp (UTC date), empty when it never expires.
fn expire_line(info: &flamix_core::model::TrafficInfo) -> String {
    let Some(seconds) = info.expire.filter(|s| *s > 0) else {
        return String::new();
    };
    chrono::DateTime::from_timestamp(seconds, 0)
        .map(|dt| format!("до {}", dt.format("%d.%m.%Y")))
        .unwrap_or_default()
}

/// Cards for the servers tab. `updating` = ids with a refresh in flight. A non-empty `filter`
/// (case-insensitive) keeps only matching servers and drops cards left without any match.
pub fn sub_cards(state: &AppState, updating: &[String], filter: &str) -> Vec<SubCard> {
    let needle = filter.trim().to_lowercase();
    let groups = exit_groups(state);
    let matches = |r: &ServerRow| {
        needle.is_empty()
            || [&r.name, &r.subtitle, &r.address]
                .iter()
                .any(|t| t.to_lowercase().contains(&needle))
    };
    let mut cards = Vec::new();
    for sub in &state.subscriptions {
        let servers: Vec<ServerRow> = sub
            .servers
            .iter()
            .map(|s| row(state, s, &groups))
            .filter(|r| matches(r))
            .collect();
        if !needle.is_empty() && servers.is_empty() && !sub.name.to_lowercase().contains(&needle) {
            continue;
        }
        let (support_url, support_kind) = support_link(sub);
        let banner = banner_view(sub);
        // The plain announce line is hidden when the banner already shows the same text.
        let announce = sub.announce.clone().unwrap_or_default();
        let announce = match &banner {
            Some(b) if b.text.trim() == announce.trim() => String::new(),
            _ => announce,
        };
        let error = if sub.provider.hwid.is_some_and(|h| h.max_devices_reached) {
            "Достигнут лимит устройств".to_string()
        } else {
            sub.last_error.clone().unwrap_or_default()
        };
        cards.push(SubCard {
            id: sub.id.clone(),
            name: if sub.name.is_empty() {
                sub.auto_title.clone().unwrap_or_else(|| sub.url.clone())
            } else {
                sub.name.clone()
            },
            local: false,
            collapsed: sub.collapsed,
            pinned: sub.pinned,
            updating: updating.contains(&sub.id),
            traffic: sub.traffic.map(|t| {
                let (text, fraction) = traffic_line(&t);
                (text, fraction, expire_line(&t))
            }),
            banner,
            announce,
            error,
            support_url,
            support_kind,
            web_url: sub
                .web_page_url
                .as_deref()
                .and_then(safe_url)
                .unwrap_or_default(),
            servers,
        });
    }
    let local: Vec<ServerRow> = state
        .local_servers
        .iter()
        .map(|s| row(state, s, &groups))
        .filter(|r| matches(r))
        .collect();
    if !local.is_empty() {
        cards.push(SubCard {
            id: String::new(),
            name: "Добавленные вручную".into(),
            local: true,
            collapsed: false,
            pinned: false,
            updating: false,
            traffic: None,
            banner: None,
            announce: String::new(),
            error: String::new(),
            support_url: String::new(),
            support_kind: SUPPORT_NONE,
            web_url: String::new(),
            servers: local,
        });
    }
    // Pinned subscriptions first; the rest keep their order (the local card stays last).
    cards.sort_by_key(|c| !c.pinned);
    cards
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

/// Stores a refreshed subscription in place of the one with the same id. The UI-only flags
/// (pinned, collapsed) are kept from the stored copy, like the desktop controller does.
pub fn apply_fetched(dir: &Path, mut sub: Subscription) -> Result<AppState, String> {
    update_state(dir, |state| {
        if let Some(existing) = state.subscriptions.iter_mut().find(|s| s.id == sub.id) {
            sub.pinned = existing.pinned;
            sub.collapsed = existing.collapsed;
            *existing = sub;
        }
    })
}

/// Changes one subscription in the stored state (`f` gets the stored subscription).
pub fn edit_sub(
    dir: &Path,
    id: &str,
    f: impl FnOnce(&mut Subscription),
) -> Result<AppState, String> {
    update_state(dir, |state| {
        if let Some(sub) = state.subscriptions.iter_mut().find(|s| s.id == id) {
            f(sub);
        }
    })
}

/// Removes a subscription and its servers. A selection pointing into it is cleared.
pub fn remove_subscription(dir: &Path, id: &str) -> Result<AppState, String> {
    update_state(dir, |state| {
        let owned: Vec<String> = state
            .subscriptions
            .iter()
            .filter(|s| s.id == id)
            .flat_map(|s| s.servers.iter().map(|x| x.id.clone()))
            .collect();
        state.subscriptions.retain(|s| s.id != id);
        if state
            .selected_server
            .as_deref()
            .is_some_and(|sel| owned.iter().any(|o| o == sel))
        {
            state.selected_server = None;
        }
    })
}

/// One non-empty, trimmed line per backup URL (the "Резервные ссылки" box).
pub fn parse_backup_urls(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect()
}

/// Result line for a share-link import.
pub fn links_status(added: usize, skipped: usize) -> String {
    if added == 0 && skipped == 0 {
        return "Все серверы уже добавлены".into();
    }
    let mut text = format!("Добавлено серверов: {added}");
    if skipped > 0 {
        text.push_str(&format!(", пропущено: {skipped}"));
    }
    text
}

/// What a pasted or clipboard text turned out to be.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Imported {
    /// A new subscription was stored; the caller fetches it next.
    Subscription { id: String, name: String },
    /// Local servers were added from share links.
    Links { added: usize, skipped: usize },
    /// Provider routing of the subscription that owns the selected server.
    Routing { enabled: bool },
    /// Routing link while no subscription is selected.
    Unsupported,
}

/// Imports a deep link, a subscription URL or share links (desktop `apply_import`, phone side).
/// Saves the state and returns the outcome together with the new state.
pub fn import_text(dir: &Path, text: &str) -> Result<(Imported, AppState), String> {
    let import =
        deeplink::parse_import(text).ok_or_else(|| "Не удалось распознать ссылку".to_string())?;
    let mut state = store::load(dir).map_err(|e| e.to_string())?;
    let outcome = match import {
        deeplink::Import::Subscription { url, name } => {
            let mut sub = new_subscription(&url)?;
            if state.subscriptions.iter().any(|s| s.url == sub.url) {
                return Err("Эта подписка уже добавлена".into());
            }
            sub.name = name.unwrap_or_default();
            let outcome = Imported::Subscription {
                id: sub.id.clone(),
                name: sub.name.clone(),
            };
            state.subscriptions.push(sub);
            outcome
        }
        deeplink::Import::Links(text) => {
            let (servers, errors) = link::parse_links(&text, "local");
            let mut added = 0;
            for server in servers {
                if state.local_servers.iter().any(|s| s.id == server.id) {
                    continue;
                }
                state.local_servers.push(server);
                added += 1;
            }
            Imported::Links {
                added,
                skipped: errors.len(),
            }
        }
        deeplink::Import::Routing(action) => apply_routing(&mut state, action),
    };
    store::save(dir, &state).map_err(|e| e.to_string())?;
    Ok((outcome, state))
}

/// Applies a `happ://routing` action to the subscription of the selected server.
fn apply_routing(state: &mut AppState, action: RoutingLinkAction) -> Imported {
    let owner = state.selected_server.as_deref().and_then(|id| {
        state
            .subscriptions
            .iter()
            .position(|sub| sub.servers.iter().any(|s| s.id == id))
    });
    let Some(index) = owner else {
        return Imported::Unsupported;
    };
    let sub = &mut state.subscriptions[index];
    match action {
        RoutingLinkAction::Add(routing) => sub.provider.routing = Some(routing),
        RoutingLinkAction::AddAndActivate(mut routing) => {
            routing.enabled = true;
            sub.provider.routing = Some(routing);
        }
        RoutingLinkAction::Off => {
            if let Some(routing) = &mut sub.provider.routing {
                routing.enabled = false;
            }
        }
    }
    Imported::Routing {
        enabled: sub.provider.routing.as_ref().is_some_and(|r| r.enabled),
    }
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
    fn backup_lines_are_trimmed_and_non_empty() {
        let parsed = parse_backup_urls("  https://a/1 \n\n https://b/2\r\n");
        assert_eq!(parsed, ["https://a/1", "https://b/2"]);
        assert_eq!(links_status(2, 1), "Добавлено серверов: 2, пропущено: 1");
        assert_eq!(links_status(0, 0), "Все серверы уже добавлены");
    }

    #[test]
    fn import_text_recognises_subscription_and_rejects_garbage() {
        let dir = std::env::temp_dir().join(format!("flamix-import-test-{}", random_hex(4)));
        std::fs::create_dir_all(&dir).unwrap();
        let (outcome, state) =
            import_text(&dir, "https://example.com/sub?x=1").expect("subscription");
        assert!(matches!(outcome, Imported::Subscription { .. }));
        assert_eq!(state.subscriptions.len(), 1);
        assert!(import_text(&dir, "hello world").is_err());
        std::fs::remove_dir_all(&dir).ok();
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

    #[test]
    fn safe_url_accepts_only_expected_schemes() {
        assert!(safe_url("https://example.com/pay").is_some());
        assert!(safe_url("tg://resolve?domain=x").is_some());
        assert!(safe_url("mailto:a@b.c").is_some());
        assert!(safe_url("javascript:alert(1)").is_none());
        assert!(safe_url("https://exa mple.com").is_none());
        assert!(safe_url("https://").is_none());
    }

    #[test]
    fn banner_tones_and_expiry_priority() {
        assert_eq!(banner_tone(Some("red")), (TONE_RED, None));
        assert_eq!(
            banner_tone(Some("#0f0")),
            (TONE_CUSTOM, Some([0, 255, 0, 255]))
        );
        assert_eq!(banner_tone(Some("nonsense")), (TONE_INFO, None));
        let mut sub = new_subscription("https://example.com/sub").unwrap();
        sub.provider.info_banner = Some(flamix_core::model::InfoBanner {
            text: "Информация".into(),
            color: Some("green".into()),
            button_text: None,
            button_link: Some("https://example.com/x".into()),
        });
        let banner = banner_view(&sub).expect("info banner");
        assert_eq!(
            (banner.tone, banner.button_text.as_str()),
            (TONE_GREEN, "Открыть")
        );
        sub.provider.expire_notice = Some("Подписка истекла".into());
        assert_eq!(banner_view(&sub).expect("expiry").tone, TONE_WARNING);
    }

    #[test]
    fn sub_cards_filter_and_quota_text() {
        let mut st = AppState::default();
        let mut sub = new_subscription("https://example.com/sub").unwrap();
        sub.name = "Main".into();
        sub.traffic = Some(flamix_core::model::TrafficInfo {
            upload: 0,
            download: 1024 * 1024,
            total: Some(10 * 1024 * 1024),
            expire: Some(1_798_761_600),
        });
        let mut a = srv("a", "a.example");
        a.name = "Alpha".into();
        a.traffic = sub.traffic;
        sub.servers = vec![a, srv("b", "b.example")];
        st.subscriptions.push(sub);
        st.local_servers.push(srv("c", "c.example"));

        let all = sub_cards(&st, &[], "");
        assert_eq!(all.len(), 2, "subscription and local card");
        let card = &all[0];
        let (text, fraction, expire) = card.traffic.clone().expect("traffic");
        assert_eq!(text, "осталось 9.0 MB из 10 MB");
        assert!((fraction - 0.1).abs() < 0.001);
        assert_eq!(expire, "до 01.01.2027");
        assert_eq!(
            card.servers[0].quota.as_ref().map(|q| q.1.as_str()),
            Some("осталось 9.0 MB")
        );

        let found = sub_cards(&st, &[], "alpha");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].servers.len(), 1);
        assert!(sub_cards(&st, &[], "zzz").is_empty());
        assert!(!sub_cards(&st, &["x".into()], "")[0].updating);
    }
}

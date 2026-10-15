//! Fetching subscriptions: URL fallback order, request headers, per-server traffic.

use super::body::decode_body;
use super::meta::{parse_meta, split_body_routing};
use super::{FetchOptions, SubscriptionMeta, DEFAULT_USER_AGENT};
use crate::error::Error;
use crate::link;
use crate::model::{
    Credentials, HwidStatus, Server, Subscription, SubscriptionOptions, TrafficInfo,
};
use crate::Result;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use std::collections::HashMap;
use std::time::Duration;

/// URLs to try, in order: primary, last working URL, then each backup URL (deduplicated).
pub fn url_order(sub: &Subscription) -> Vec<String> {
    let mut urls: Vec<String> = Vec::new();
    for url in std::iter::once(&sub.url)
        .chain(sub.last_working_url.iter())
        .chain(sub.backup_urls.iter())
    {
        let url = url.trim();
        if !url.is_empty() && !urls.iter().any(|seen| seen == url) {
            urls.push(url.to_string());
        }
    }
    urls
}

/// Download `sub` (primary URL, then `last_working_url`, then each backup URL, until one
/// answers 2xx with a non-empty server list) and update it in place: servers (keeping
/// `last_ping_ms` / `local_traffic` of servers whose id did not change), meta, `last_update`,
/// `last_error`, `last_working_url`. New backup URLs from headers are merged in (deduplicated).
pub async fn update(sub: &mut Subscription, opts: &FetchOptions) -> Result<()> {
    let client = build_client(opts)?;
    let urls = url_order(sub);

    let mut errors: Vec<String> = Vec::new();
    let mut success: Option<(String, FetchOutcome)> = None;
    for url in &urls {
        match fetch_one(&client, url, sub, opts).await {
            Ok(outcome) if !outcome.servers.is_empty() => {
                success = Some((url.clone(), outcome));
                break;
            }
            Ok(_) => errors.push(format!("{url}: no servers")),
            Err(err) => {
                // Even on an error response the panel may report the HWID device limit,
                // so the UI can explain why the subscription is unavailable.
                if err.hwid.is_some() {
                    sub.provider.hwid = err.hwid;
                }
                errors.push(format!("{url}: {}", err.error));
            }
        }
    }

    let Some((working_url, outcome)) = success else {
        let message = if errors.is_empty() {
            "no subscription URL".to_string()
        } else {
            errors.join("; ")
        };
        sub.last_error = Some(short_error(&message));
        return Err(Error::Http(message));
    };
    let FetchOutcome { mut servers, meta } = outcome;

    let previous: HashMap<String, (Option<Option<u32>>, TrafficInfo)> = sub
        .servers
        .iter()
        .map(|s| (s.id.clone(), (s.last_ping_ms, s.local_traffic)))
        .collect();
    for server in &mut servers {
        if let Some((ping, local)) = previous.get(&server.id) {
            server.last_ping_ms = *ping;
            server.local_traffic = *local;
        }
    }

    apply_server_userinfo(&mut servers, &meta.server_userinfo);
    apply_filters(&mut servers, &sub.options, meta.network_filter.as_deref());
    sub.servers = servers;

    apply_provider_extras(sub, &meta, &working_url);

    if let Some(info) = meta.userinfo {
        sub.traffic = Some(info);
    }
    if let Some(title) = meta.title {
        let is_auto = sub.name.is_empty() || sub.auto_title.as_deref() == Some(sub.name.as_str());
        if is_auto {
            sub.name = title.clone();
        }
        sub.auto_title = Some(title);
    }
    if let Some(url) = meta.support_url {
        sub.support_url = Some(url);
    }
    if let Some(url) = meta.web_page_url {
        sub.web_page_url = Some(url);
    }
    if let Some(announce) = meta.announce {
        sub.announce = Some(announce);
    }
    if sub.update_interval_hours == 0 {
        if let Some(hours) = meta.update_interval_hours {
            sub.update_interval_hours = hours;
        }
    }

    merge_backup_urls(sub, &meta.backup_urls);

    sub.last_working_url = Some(working_url);
    sub.last_update = Some(chrono::Utc::now().timestamp());
    sub.last_error = None;
    Ok(())
}

/// Store every provider extra (routing, banners, HWID, overrides, ...) into `sub.provider`.
fn apply_provider_extras(sub: &mut Subscription, meta: &SubscriptionMeta, working_url: &str) {
    if let Some(id) = &meta.provider_id {
        sub.provider.provider_id = Some(id.clone());
    } else if sub.provider.provider_id.is_none() {
        sub.provider.provider_id = provider_id_from_url(working_url);
    }

    // Keep a routing profile the user disabled, unless `routing-enable` says otherwise.
    if let Some(mut routing) = meta.routing.clone() {
        if let Some(old) = &sub.provider.routing {
            if !old.enabled {
                routing.enabled = false;
            }
        }
        if let Some(enable) = meta.routing_enable {
            routing.enabled = enable;
        }
        sub.provider.routing = Some(routing);
    } else if let Some(enable) = meta.routing_enable {
        if let Some(routing) = &mut sub.provider.routing {
            routing.enabled = enable;
        }
    }

    if let Some(url) = &meta.announce_url {
        sub.provider.announce_url = Some(url.clone());
    }
    if let Some(banner) = &meta.info_banner {
        sub.provider.info_banner = Some(banner.clone());
    }
    if let Some(text) = &meta.expire_notice {
        sub.provider.expire_notice = Some(text.clone());
    }
    if let Some(link) = &meta.expire_button_link {
        sub.provider.expire_button_link = Some(link.clone());
    }
    sub.provider.update_always = meta.update_always;
    sub.provider.network_filter = meta.network_filter.clone();
    sub.provider.refill_date = meta.refill_date;
    sub.provider.hwid = meta.hwid;
    for (k, v) in &meta.app_overrides {
        sub.provider.app_overrides.insert(k.clone(), v.clone());
    }
}

/// `providerid` from the subscription URL fragment: `#?providerid=X` (or `#providerid=X`).
fn provider_id_from_url(url: &str) -> Option<String> {
    let fragment = url.split_once('#')?.1;
    let query = fragment.strip_prefix('?').unwrap_or(fragment);
    url::form_urlencoded::parse(query.as_bytes())
        .find(|(k, _)| k.eq_ignore_ascii_case("providerid"))
        .map(|(_, v)| v.into_owned())
        .filter(|v| !v.is_empty())
}

/// Keep only servers passing the user's `name_filter` regex and the provider's `network_filter`.
/// The network filter keeps servers whose transport network or protocol name matches any
/// comma-separated token; an empty value (or a boolean toggle) applies no filter.
pub fn apply_filters(
    servers: &mut Vec<Server>,
    options: &SubscriptionOptions,
    network_filter: Option<&str>,
) {
    if let Some(pattern) = options
        .name_filter
        .as_deref()
        .map(str::trim)
        .filter(|p| !p.is_empty())
    {
        match regex::Regex::new(pattern) {
            Ok(re) => servers.retain(|server| re.is_match(&server.name)),
            Err(err) => tracing::warn!(pattern, %err, "invalid name_filter, ignored"),
        }
    }

    let Some(filter) = network_filter.map(str::trim).filter(|f| !f.is_empty()) else {
        return;
    };
    if matches!(
        filter.to_ascii_lowercase().as_str(),
        "true" | "1" | "false" | "0"
    ) {
        return;
    }
    let tokens: Vec<String> = filter
        .split(',')
        .map(|t| t.trim().to_ascii_lowercase())
        .filter(|t| !t.is_empty())
        .collect();
    if tokens.is_empty() {
        return;
    }
    servers.retain(|server| {
        let network = server.transport.network.as_str();
        let protocol = server.protocol().as_str();
        tokens.iter().any(|t| t == network || t == protocol)
    });
}

/// Apply per-server quota from the `server-userinfo` header. Keys are matched against each
/// server as: exact name (remark) → `address:port` → credential id (uuid / password). The
/// header value wins over a `userinfo=` parameter already stored in [`Server::traffic`].
pub fn apply_server_userinfo(servers: &mut [Server], map: &HashMap<String, TrafficInfo>) {
    if map.is_empty() {
        return;
    }
    for server in servers {
        let mut hit = map.get(&server.name);
        if hit.is_none() {
            hit = map.get(&format!("{}:{}", server.address, server.port));
        }
        if hit.is_none() {
            if let Some(id) = credential_id(server) {
                hit = map.get(id);
            }
        }
        if let Some(info) = hit {
            server.traffic = Some(*info);
        }
    }
}

fn credential_id(server: &Server) -> Option<&str> {
    match &server.credentials {
        Credentials::Vless { uuid, .. } => Some(uuid),
        Credentials::Vmess { uuid, .. } => Some(uuid),
        Credentials::Tuic { uuid, .. } => Some(uuid),
        Credentials::Trojan { password } => Some(password),
        Credentials::Shadowsocks { password, .. } => Some(password),
        Credentials::Hysteria2 { password, .. } => Some(password),
        _ => None,
    }
}

fn merge_backup_urls(sub: &mut Subscription, extra: &[String]) {
    let primary = sub.url.clone();
    for url in extra {
        if url != &primary && !sub.backup_urls.contains(url) {
            sub.backup_urls.push(url.clone());
        }
    }
    let mut seen: Vec<String> = Vec::new();
    let primary = sub.url.clone();
    sub.backup_urls.retain(|url| {
        url != &primary && !seen.iter().any(|s| s == url) && {
            seen.push(url.clone());
            true
        }
    });
    sub.backup_urls.truncate(10);
}

fn short_error(message: &str) -> String {
    message.chars().take(500).collect()
}

fn build_client(opts: &FetchOptions) -> Result<reqwest::Client> {
    let timeout = opts.timeout.unwrap_or(Duration::from_secs(20));
    let mut builder = reqwest::Client::builder()
        .timeout(timeout)
        .redirect(reqwest::redirect::Policy::limited(10));
    if let Some(proxy) = &opts.proxy {
        builder = builder
            .proxy(reqwest::Proxy::all(proxy).map_err(|e| Error::Http(format!("bad proxy: {e}")))?);
    }
    builder.build().map_err(|e| Error::Http(e.to_string()))
}

fn request_headers(sub: &Subscription, opts: &FetchOptions) -> HeaderMap {
    let mut headers = HeaderMap::new();
    let user_agent = sub.user_agent.as_deref().unwrap_or(DEFAULT_USER_AGENT);
    if let Ok(value) = HeaderValue::from_str(user_agent) {
        headers.insert(reqwest::header::USER_AGENT, value);
    }
    if let Some(hwid) = &opts.hwid {
        if let Ok(value) = HeaderValue::from_str(hwid) {
            headers.insert(HeaderName::from_static("x-hwid"), value);
        }
    }
    if let Ok(value) = HeaderValue::from_str(std::env::consts::OS) {
        headers.insert(HeaderName::from_static("x-device-os"), value);
    }
    if let Ok(value) = HeaderValue::from_str(&os_version()) {
        headers.insert(HeaderName::from_static("x-ver-os"), value);
    }
    if let Ok(value) = HeaderValue::from_str(&device_model()) {
        headers.insert(HeaderName::from_static("x-device-model"), value);
    }
    // User headers override the defaults (but never the `Host` header).
    for (name, value) in &sub.options.custom_headers {
        if name.eq_ignore_ascii_case("host") {
            continue;
        }
        if let (Ok(name), Ok(value)) = (
            HeaderName::from_bytes(name.as_bytes()),
            HeaderValue::from_str(value),
        ) {
            headers.insert(name, value);
        }
    }
    headers
}

fn os_version() -> String {
    std::env::var("FLAMIX_OS_VERSION").unwrap_or_default()
}

fn device_model() -> String {
    std::env::var("FLAMIX_DEVICE_MODEL").unwrap_or_else(|_| "Desktop".to_string())
}

/// A successful fetch: parsed servers plus the subscription metadata.
struct FetchOutcome {
    servers: Vec<Server>,
    meta: SubscriptionMeta,
}

/// A failed fetch, keeping the HWID state the panel may have reported.
struct FetchError {
    error: Error,
    hwid: Option<HwidStatus>,
}

impl FetchError {
    fn plain(error: Error) -> Self {
        Self { error, hwid: None }
    }
}

fn collect_headers(response: &reqwest::Response) -> Vec<(String, String)> {
    response
        .headers()
        .iter()
        .map(|(name, value)| {
            (
                name.as_str().to_string(),
                value.to_str().unwrap_or("").to_string(),
            )
        })
        .collect()
}

async fn fetch_one(
    client: &reqwest::Client,
    url: &str,
    sub: &Subscription,
    opts: &FetchOptions,
) -> std::result::Result<FetchOutcome, FetchError> {
    let response = client
        .get(url)
        .headers(request_headers(sub, opts))
        .send()
        .await
        .map_err(|e| FetchError::plain(Error::Http(e.to_string())))?;
    let headers = collect_headers(&response);
    if !response.status().is_success() {
        // Read `x-hwid-*` even on an error response (device-limit reporting).
        let meta = parse_meta(&headers, "");
        return Err(FetchError {
            error: Error::Http(format!("HTTP {}", response.status().as_u16())),
            hwid: meta.hwid,
        });
    }
    let body = response
        .text()
        .await
        .map_err(|e| FetchError::plain(Error::Http(e.to_string())))?;

    // A raw `happ://routing/...` body line is metadata, not a share link: pull it out first.
    let (body_routing, body_for_meta) = split_body_routing(&body);
    let mut meta = parse_meta(&headers, &body_for_meta);
    if meta.routing.is_none() {
        meta.routing = body_routing;
    }
    let text = decode_body(&body_for_meta).map_err(FetchError::plain)?;
    let (servers, _errors) = link::parse_links(&text, &sub.id);
    Ok(FetchOutcome { servers, meta })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Network, ProviderRouting, Security, SubscriptionOptions};
    use std::sync::{Arc, Mutex};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    fn traffic(download: u64, total: u64) -> TrafficInfo {
        TrafficInfo {
            upload: 0,
            download,
            total: Some(total),
            expire: None,
        }
    }

    fn test_server(name: &str, address: &str, port: u16, credentials: Credentials) -> Server {
        Server {
            id: name.to_string(),
            name: name.to_string(),
            country: None,
            address: address.to_string(),
            port,
            credentials,
            transport: Default::default(),
            security: Security::None,
            raw_link: None,
            traffic: None,
            local_traffic: TrafficInfo::default(),
            last_ping_ms: None,
        }
    }

    fn test_sub(url: &str) -> Subscription {
        Subscription {
            id: "sub".to_string(),
            name: String::new(),
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
            auto_title: None,
            provider: Default::default(),
            options: Default::default(),
        }
    }

    #[test]
    fn url_order_is_deduped() {
        let mut sub = test_sub("http://a/primary");
        sub.last_working_url = Some("http://a/last".to_string());
        sub.backup_urls = vec![
            "http://a/primary".to_string(),
            "http://a/b1".to_string(),
            "http://a/last".to_string(),
        ];
        assert_eq!(
            url_order(&sub),
            vec!["http://a/primary", "http://a/last", "http://a/b1"]
        );
    }

    #[test]
    fn server_userinfo_matches_name_host_uuid_password() {
        let uuid = "b831381d-6324-4d53-ad4f-8cda48b30811".to_string();
        let mut servers = vec![
            test_server(
                "DE #1",
                "de1.example.com",
                443,
                Credentials::Vless {
                    uuid: uuid.clone(),
                    flow: None,
                    encryption: None,
                },
            ),
            test_server(
                "Trojan",
                "tr.example.com",
                443,
                Credentials::Trojan {
                    password: "secret".to_string(),
                },
            ),
            test_server(
                "SS",
                "ss.example.com",
                8388,
                Credentials::Shadowsocks {
                    method: "aes-256-gcm".to_string(),
                    password: "sspw".to_string(),
                    plugin: None,
                    plugin_opts: None,
                },
            ),
        ];

        let mut map = HashMap::new();
        map.insert("DE #1".to_string(), traffic(1, 10));
        map.insert("tr.example.com:443".to_string(), traffic(2, 20));
        map.insert("sspw".to_string(), traffic(3, 30));
        apply_server_userinfo(&mut servers, &map);

        assert_eq!(servers[0].traffic.unwrap().total, Some(10));
        assert_eq!(servers[1].traffic.unwrap().total, Some(20));
        assert_eq!(servers[2].traffic.unwrap().total, Some(30));
    }

    #[test]
    fn server_userinfo_name_wins_over_uuid() {
        let uuid = "uuid-1".to_string();
        let mut servers = vec![test_server(
            "A",
            "h",
            443,
            Credentials::Vless {
                uuid: uuid.clone(),
                flow: None,
                encryption: None,
            },
        )];
        let mut map = HashMap::new();
        map.insert("A".to_string(), traffic(1, 10));
        map.insert(uuid, traffic(2, 20));
        apply_server_userinfo(&mut servers, &map);
        assert_eq!(servers[0].traffic.unwrap().total, Some(10));
    }

    async fn serve(
        listener: TcpListener,
        mapping: HashMap<String, (u16, String)>,
        requests: Arc<Mutex<Vec<String>>>,
    ) {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                break;
            };
            let mut buf = vec![0u8; 8192];
            let n = stream.read(&mut buf).await.unwrap_or(0);
            let request = String::from_utf8_lossy(&buf[..n]).to_string();
            let path = request
                .lines()
                .next()
                .and_then(|line| line.split_whitespace().nth(1))
                .unwrap_or("/")
                .to_string();
            requests.lock().unwrap().push(request);
            let (status, body) = mapping.get(&path).cloned().unwrap_or((404, String::new()));
            let reason = match status {
                200 => "OK",
                500 => "Internal Server Error",
                _ => "Not Found",
            };
            let response = format!(
                "HTTP/1.1 {status} {reason}\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes()).await;
            let _ = stream.shutdown().await;
        }
    }

    async fn start_server(
        mapping: HashMap<String, (u16, String)>,
    ) -> (String, Arc<Mutex<Vec<String>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let handle = requests.clone();
        tokio::spawn(serve(listener, mapping, handle));
        (base, requests)
    }

    #[tokio::test]
    async fn update_tries_urls_in_order_and_keeps_old_servers_on_failure() {
        let mapping: HashMap<String, (u16, String)> = [
            ("/primary".to_string(), (500, String::new())),
            ("/last".to_string(), (200, "not a subscription".to_string())),
            ("/b1".to_string(), (200, String::new())),
        ]
        .into();
        let (base, requests) = start_server(mapping).await;

        let mut sub = test_sub(&format!("{base}/primary"));
        sub.last_working_url = Some(format!("{base}/last"));
        sub.backup_urls = vec![format!("{base}/b1")];
        sub.servers = vec![test_server(
            "keep",
            "h",
            443,
            Credentials::Trojan {
                password: "p".to_string(),
            },
        )];

        let err = update(&mut sub, &FetchOptions::default()).await;
        assert!(matches!(err, Err(Error::Http(_))));
        assert!(sub.last_error.is_some());
        assert_eq!(sub.servers.len(), 1);
        assert_eq!(sub.servers[0].name, "keep");

        let requests = requests.lock().unwrap().clone();
        let paths: Vec<String> = requests
            .iter()
            .filter_map(|r| {
                r.lines()
                    .next()?
                    .split_whitespace()
                    .nth(1)
                    .map(str::to_string)
            })
            .collect();
        assert_eq!(paths, vec!["/primary", "/last", "/b1"]);

        let first = requests[0].to_lowercase();
        assert!(first.contains("user-agent: flamix/"));
        assert!(first.contains("x-device-os:"));
    }

    #[tokio::test]
    async fn update_sends_hwid_header() {
        let mapping: HashMap<String, (u16, String)> =
            [("/x".to_string(), (500, String::new()))].into();
        let (base, requests) = start_server(mapping).await;
        let mut sub = test_sub(&format!("{base}/x"));
        let opts = FetchOptions {
            hwid: Some("deadbeef".to_string()),
            ..FetchOptions::default()
        };
        let _ = update(&mut sub, &opts).await;
        assert!(requests.lock().unwrap()[0]
            .to_lowercase()
            .contains("x-hwid: deadbeef"));
    }

    #[tokio::test]
    #[ignore = "needs T01 link parser"]
    async fn update_falls_back_to_backup_with_links() {
        let links = "vless://uuid@h:443#A\n";
        let mapping: HashMap<String, (u16, String)> = [
            ("/primary".to_string(), (500, String::new())),
            ("/b1".to_string(), (200, String::new())),
            ("/b2".to_string(), (200, links.to_string())),
        ]
        .into();
        let (base, _requests) = start_server(mapping).await;

        let mut sub = test_sub(&format!("{base}/primary"));
        sub.backup_urls = vec![format!("{base}/b1"), format!("{base}/b2")];
        sub.name = "provider".to_string();

        update(&mut sub, &FetchOptions::default()).await.unwrap();
        assert_eq!(sub.last_working_url, Some(format!("{base}/b2")));
        assert_eq!(sub.servers.len(), 1);
        assert_eq!(sub.servers[0].name, "A");
    }

    #[test]
    fn name_filter_and_network_filter() {
        let mut servers = vec![
            test_server(
                "DE ws",
                "h1",
                443,
                Credentials::Trojan {
                    password: "p".to_string(),
                },
            ),
            test_server(
                "US grpc",
                "h2",
                443,
                Credentials::Trojan {
                    password: "p".to_string(),
                },
            ),
            test_server(
                "NL tcp",
                "h3",
                443,
                Credentials::Trojan {
                    password: "p".to_string(),
                },
            ),
        ];
        servers[0].transport.network = Network::Ws;
        servers[1].transport.network = Network::Grpc;

        let mut opts = SubscriptionOptions {
            name_filter: Some("^(DE|US)".to_string()),
            ..SubscriptionOptions::default()
        };
        let mut filtered = servers.clone();
        apply_filters(&mut filtered, &opts, None);
        assert_eq!(filtered.len(), 2);

        // Network filter keeps only matching transports (comma separated, case-insensitive).
        let mut by_network = servers.clone();
        apply_filters(
            &mut by_network,
            &SubscriptionOptions::default(),
            Some("WS, tcp"),
        );
        assert_eq!(by_network.len(), 2);
        assert_eq!(by_network[0].name, "DE ws");
        assert_eq!(by_network[1].name, "NL tcp");

        // Boolean toggles and empty values apply no filter.
        let mut all = servers.clone();
        apply_filters(&mut all, &SubscriptionOptions::default(), Some("true"));
        assert_eq!(all.len(), 3);
        apply_filters(&mut all, &SubscriptionOptions::default(), Some("  "));
        assert_eq!(all.len(), 3);

        // A bad regex is ignored, not fatal.
        opts.name_filter = Some("([".to_string());
        let mut kept = servers;
        apply_filters(&mut kept, &opts, None);
        assert_eq!(kept.len(), 3);
    }

    #[tokio::test]
    async fn custom_headers_are_sent_and_host_ignored() {
        let mapping: HashMap<String, (u16, String)> =
            [("/x".to_string(), (500, String::new()))].into();
        let (base, requests) = start_server(mapping).await;
        let mut sub = test_sub(&format!("{base}/x"));
        sub.options
            .custom_headers
            .insert("X-Custom".to_string(), "abc".to_string());
        sub.options
            .custom_headers
            .insert("User-Agent".to_string(), "MyAgent/1".to_string());
        sub.options
            .custom_headers
            .insert("Host".to_string(), "evil.example".to_string());
        let _ = update(&mut sub, &FetchOptions::default()).await;

        let request = requests.lock().unwrap()[0].to_lowercase();
        assert!(request.contains("x-custom: abc"));
        assert!(request.contains("user-agent: myagent/1"));
        assert!(!request.contains("evil.example"));
    }

    #[tokio::test]
    async fn hwid_headers_read_on_error_response() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move {
            if let Ok((mut stream, _)) = listener.accept().await {
                let mut buf = [0u8; 4096];
                let _ = stream.read(&mut buf).await;
                let response = "HTTP/1.1 403 Forbidden\r\nx-hwid-limit: 3\r\nx-hwid-active: 3\r\nx-hwid-max-devices-reached: true\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
                let _ = stream.write_all(response.as_bytes()).await;
                let _ = stream.shutdown().await;
            }
        });
        let mut sub = test_sub(&format!("{base}/sub"));
        let err = update(&mut sub, &FetchOptions::default()).await;
        assert!(err.is_err());
        let hwid = sub.provider.hwid.expect("hwid headers should be read");
        assert_eq!(hwid.limit, Some(3));
        assert!(hwid.max_devices_reached);
        assert!(!hwid.not_supported);
    }

    #[test]
    fn provider_id_from_url_fragment() {
        assert_eq!(
            provider_id_from_url("https://x.com/sub/123#?providerid=Abcd1234").as_deref(),
            Some("Abcd1234")
        );
        assert_eq!(
            provider_id_from_url("https://x.com/sub#providerid=Zz").as_deref(),
            Some("Zz")
        );
        assert_eq!(provider_id_from_url("https://x.com/sub"), None);
    }

    #[test]
    fn provider_extras_stored_and_disabled_routing_kept() {
        let mut sub = test_sub("https://x/sub");
        sub.provider.routing = Some(ProviderRouting {
            name: "old".to_string(),
            enabled: false,
            ..ProviderRouting::default()
        });

        let meta = SubscriptionMeta {
            routing: Some(ProviderRouting {
                name: "new".to_string(),
                enabled: true,
                ..ProviderRouting::default()
            }),
            announce_url: Some("https://t.me/x".to_string()),
            network_filter: Some("tcp".to_string()),
            update_always: true,
            ..SubscriptionMeta::default()
        };
        apply_provider_extras(&mut sub, &meta, "https://x/sub/123#?providerid=Abcd1234");

        assert_eq!(sub.provider.provider_id.as_deref(), Some("Abcd1234"));
        assert_eq!(sub.provider.announce_url.as_deref(), Some("https://t.me/x"));
        assert_eq!(sub.provider.network_filter.as_deref(), Some("tcp"));
        assert!(sub.provider.update_always);
        // The user disabled routing, so `enabled` stays false even with a new profile.
        let routing = sub.provider.routing.as_ref().unwrap();
        assert_eq!(routing.name, "new");
        assert!(!routing.enabled);

        // `routing-enable: true` re-enables it.
        let meta = SubscriptionMeta {
            routing_enable: Some(true),
            ..SubscriptionMeta::default()
        };
        apply_provider_extras(&mut sub, &meta, "https://x/sub");
        assert!(sub.provider.routing.unwrap().enabled);
    }
}

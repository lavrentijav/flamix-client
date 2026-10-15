//! Fetching subscriptions: URL fallback order, request headers, per-server traffic.

use super::body::decode_body;
use super::meta::parse_meta;
use super::{FetchOptions, SubscriptionMeta, DEFAULT_USER_AGENT};
use crate::error::Error;
use crate::link;
use crate::model::{Credentials, Server, Subscription, TrafficInfo};
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
    let mut success: Option<(String, Vec<Server>, SubscriptionMeta)> = None;
    for url in &urls {
        match fetch_one(&client, url, sub, opts).await {
            Ok((servers, meta)) if !servers.is_empty() => {
                success = Some((url.clone(), servers, meta));
                break;
            }
            Ok(_) => errors.push(format!("{url}: no servers")),
            Err(err) => errors.push(format!("{url}: {err}")),
        }
    }

    let Some((working_url, mut servers, meta)) = success else {
        let message = if errors.is_empty() {
            "no subscription URL".to_string()
        } else {
            errors.join("; ")
        };
        sub.last_error = Some(short_error(&message));
        return Err(Error::Http(message));
    };

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
    sub.servers = servers;

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
    headers
}

fn os_version() -> String {
    std::env::var("FLAMIX_OS_VERSION").unwrap_or_default()
}

fn device_model() -> String {
    std::env::var("FLAMIX_DEVICE_MODEL").unwrap_or_else(|_| "Desktop".to_string())
}

async fn fetch_one(
    client: &reqwest::Client,
    url: &str,
    sub: &Subscription,
    opts: &FetchOptions,
) -> Result<(Vec<Server>, SubscriptionMeta)> {
    let response = client
        .get(url)
        .headers(request_headers(sub, opts))
        .send()
        .await
        .map_err(|e| Error::Http(e.to_string()))?;
    if !response.status().is_success() {
        return Err(Error::Http(format!("HTTP {}", response.status().as_u16())));
    }
    let headers: Vec<(String, String)> = response
        .headers()
        .iter()
        .map(|(name, value)| {
            (
                name.as_str().to_string(),
                value.to_str().unwrap_or("").to_string(),
            )
        })
        .collect();
    let body = response
        .text()
        .await
        .map_err(|e| Error::Http(e.to_string()))?;

    let meta = parse_meta(&headers, &body);
    let text = decode_body(&body)?;
    let (servers, _errors) = link::parse_links(&text, &sub.id);
    Ok((servers, meta))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Security;
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
}

//! Subscriptions: download, decode, headers, backup URLs. OWNER: task T02.
//! Header format: see `docs/subscription-headers.md` (authoritative).

use crate::model::{Subscription, TrafficInfo};
use crate::Result;
use std::collections::HashMap;

/// User-Agent sent by default. Many panels (Marzban, Remnawave, 3x-ui) choose the
/// response format by UA; this one makes them return plain/base64 share links.
pub const DEFAULT_USER_AGENT: &str =
    concat!("Flamix/", env!("CARGO_PKG_VERSION"), " v2rayN/7.0 Happ/3.0");

/// Metadata extracted from response headers and from `#key: value` lines at the top of the body.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SubscriptionMeta {
    pub title: Option<String>,
    pub userinfo: Option<TrafficInfo>,
    pub update_interval_hours: Option<u32>,
    pub support_url: Option<String>,
    pub web_page_url: Option<String>,
    pub announce: Option<String>,
    pub backup_urls: Vec<String>,
    /// Per-server quotas keyed as described in the spec (name, `host:port`, or uuid/password).
    pub server_userinfo: HashMap<String, TrafficInfo>,
}

/// Request options for [`update`].
#[derive(Debug, Clone, Default)]
pub struct FetchOptions {
    /// Optional local proxy URL to fetch through, e.g. "socks5h://127.0.0.1:10808".
    pub proxy: Option<String>,
    /// Stable device id sent as `x-hwid` (see docs/subscription-headers.md).
    pub hwid: Option<String>,
    /// Request timeout per URL attempt; `None` = 20 s.
    pub timeout: Option<std::time::Duration>,
}

/// Parse a `subscription-userinfo` style value: `upload=1; download=2; total=3; expire=4`.
pub fn parse_userinfo(value: &str) -> Option<TrafficInfo> {
    let _ = value;
    todo!("T02")
}

/// Extract metadata from headers (names case-insensitive) and the body.
pub fn parse_meta(headers: &[(String, String)], body: &str) -> SubscriptionMeta {
    let _ = (headers, body);
    todo!("T02")
}

/// Decode a subscription body into share links text (one link per line):
/// base64 (std/url-safe, padded or not), plain text, or JSON / Clash YAML (best effort).
pub fn decode_body(body: &str) -> Result<String> {
    let _ = body;
    todo!("T02")
}

/// Download `sub` (primary URL, then `last_working_url`, then each backup URL, until one
/// answers 2xx with a non-empty server list) and update it in place: servers (keeping
/// `last_ping_ms` / `local_traffic` of servers whose id did not change), meta, `last_update`,
/// `last_error`, `last_working_url`. New backup URLs from headers are merged in (deduplicated).
pub async fn update(sub: &mut Subscription, opts: &FetchOptions) -> Result<()> {
    let _ = (sub, opts);
    todo!("T02")
}

//! Subscriptions: download, decode, headers, backup URLs. OWNER: task T02.
//! Header format: see `docs/subscription-headers.md` (authoritative).
//!
//! The implementation is split into [`meta`] (headers / `#key: value` lines),
//! [`body`] (base64 / plain / JSON / Clash YAML decoding) and [`fetch`] (request + fallback).

mod body;
mod fetch;
mod meta;
pub(crate) mod util;

pub use body::decode_body;
pub use fetch::{apply_filters, apply_server_userinfo, update, url_order};
pub use meta::{parse_meta, parse_server_userinfo, parse_userinfo, split_body_routing};

use crate::model::{HwidStatus, InfoBanner, ProviderRouting, Timestamp, TrafficInfo};
use std::collections::{BTreeMap, HashMap};

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

    // --- compatibility extras (Happ / v2RayTun / Remnawave) ---
    /// Happ `providerid` (header, body `#providerid X` or URL fragment `#?providerid=X`).
    pub provider_id: Option<String>,
    /// Routing profile from `routing` / a raw `happ://routing/...` body line.
    pub routing: Option<ProviderRouting>,
    /// `routing-enable`: `Some(false)` when the provider explicitly disables routing.
    pub routing_enable: Option<bool>,
    /// v2RayTun `announce-url`.
    pub announce_url: Option<String>,
    /// Happ info banner (`sub-info-*`).
    pub info_banner: Option<InfoBanner>,
    /// Happ `sub-expire` raw value.
    pub expire_notice: Option<String>,
    /// Happ `sub-expire-button-link`.
    pub expire_button_link: Option<String>,
    /// v2RayTun `update-always`.
    pub update_always: bool,
    /// v2RayTun `network-filter` raw value.
    pub network_filter: Option<String>,
    /// Remnawave `subscription-refill-date` as unix seconds.
    pub refill_date: Option<Timestamp>,
    /// HWID device-limit state from `x-hwid-*` response headers.
    pub hwid: Option<HwidStatus>,
    /// Other Happ app-management keys, lower-case key -> raw value.
    pub app_overrides: BTreeMap<String, String>,
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

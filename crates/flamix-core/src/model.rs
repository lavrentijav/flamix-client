//! Data model shared by every part of Flamix.
//!
//! This file is the contract between modules: the link parser produces [`Server`]s,
//! the subscription module produces [`Subscription`]s, the config generators consume
//! [`Server`] + [`Settings`], and the store persists [`AppState`]. Keep it serde-stable:
//! new fields must be `#[serde(default)]` so old state files keep loading.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Unix timestamp in seconds (UTC).
pub type Timestamp = i64;

/// Proxy protocol of a server.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Protocol {
    Vless,
    Vmess,
    Trojan,
    Shadowsocks,
    Hysteria2,
    Tuic,
    Wireguard,
    Socks,
    Http,
}

impl Protocol {
    /// Short lowercase name used in the UI ("vless / ws / tls").
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Vless => "vless",
            Self::Vmess => "vmess",
            Self::Trojan => "trojan",
            Self::Shadowsocks => "ss",
            Self::Hysteria2 => "hysteria2",
            Self::Tuic => "tuic",
            Self::Wireguard => "wireguard",
            Self::Socks => "socks",
            Self::Http => "http",
        }
    }
}

/// Transport (stream network) for V2Ray-family protocols.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Network {
    #[default]
    Tcp,
    Ws,
    Grpc,
    HttpUpgrade,
    Xhttp,
    H2,
    Kcp,
    Quic,
}

impl Network {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Tcp => "tcp",
            Self::Ws => "ws",
            Self::Grpc => "grpc",
            Self::HttpUpgrade => "httpupgrade",
            Self::Xhttp => "xhttp",
            Self::H2 => "h2",
            Self::Kcp => "kcp",
            Self::Quic => "quic",
        }
    }
}

/// Transport parameters. Fields not used by the chosen [`Network`] stay empty.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Transport {
    pub network: Network,
    /// ws/httpupgrade/xhttp/h2 path.
    pub path: Option<String>,
    /// ws/httpupgrade/xhttp/h2 Host header (h2: comma separated list allowed).
    pub host: Option<String>,
    /// grpc serviceName.
    pub service_name: Option<String>,
    /// grpc mode ("gun"/"multi") or xhttp mode ("auto", "packet-up", "stream-up", "stream-one").
    pub mode: Option<String>,
    /// tcp header type ("none"/"http"), kcp/quic header type.
    pub header_type: Option<String>,
    /// kcp seed / quic key.
    pub seed: Option<String>,
    /// xhttp `extra` JSON object, passed through verbatim.
    pub extra: Option<serde_json::Value>,
}

/// Transport security layer.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Security {
    #[default]
    None,
    Tls {
        #[serde(default)]
        sni: Option<String>,
        #[serde(default)]
        alpn: Vec<String>,
        /// uTLS fingerprint ("chrome", "firefox", ...).
        #[serde(default)]
        fingerprint: Option<String>,
        #[serde(default)]
        allow_insecure: bool,
    },
    Reality {
        #[serde(default)]
        sni: Option<String>,
        public_key: String,
        #[serde(default)]
        short_id: Option<String>,
        #[serde(default)]
        spider_x: Option<String>,
        #[serde(default)]
        fingerprint: Option<String>,
    },
}

impl Security {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Tls { .. } => "tls",
            Self::Reality { .. } => "reality",
        }
    }
}

/// Protocol-specific credentials and options.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "protocol", rename_all = "lowercase")]
pub enum Credentials {
    Vless {
        uuid: String,
        #[serde(default)]
        flow: Option<String>,
        /// Usually "none".
        #[serde(default)]
        encryption: Option<String>,
    },
    Vmess {
        uuid: String,
        #[serde(default)]
        alter_id: u32,
        /// "auto", "aes-128-gcm", "chacha20-poly1305", "none", "zero".
        #[serde(default)]
        cipher: Option<String>,
    },
    Trojan {
        password: String,
    },
    Shadowsocks {
        method: String,
        password: String,
        /// SIP003 plugin name and options (e.g. "obfs-local", "obfs=http;obfs-host=x").
        #[serde(default)]
        plugin: Option<String>,
        #[serde(default)]
        plugin_opts: Option<String>,
    },
    Hysteria2 {
        password: String,
        #[serde(default)]
        obfs: Option<String>,
        #[serde(default)]
        obfs_password: Option<String>,
        /// Port hopping range, e.g. "20000-30000".
        #[serde(default)]
        ports: Option<String>,
        #[serde(default)]
        up_mbps: Option<u32>,
        #[serde(default)]
        down_mbps: Option<u32>,
    },
    Tuic {
        uuid: String,
        password: String,
        #[serde(default)]
        congestion_control: Option<String>,
        #[serde(default)]
        udp_relay_mode: Option<String>,
    },
    Wireguard {
        private_key: String,
        peer_public_key: String,
        #[serde(default)]
        pre_shared_key: Option<String>,
        /// Interface addresses with prefix, e.g. ["10.0.0.2/32", "fd00::2/128"].
        #[serde(default)]
        local_address: Vec<String>,
        #[serde(default)]
        reserved: Option<[u8; 3]>,
        #[serde(default)]
        mtu: Option<u32>,
    },
    Socks {
        #[serde(default)]
        username: Option<String>,
        #[serde(default)]
        password: Option<String>,
    },
    Http {
        #[serde(default)]
        username: Option<String>,
        #[serde(default)]
        password: Option<String>,
    },
}

impl Credentials {
    pub fn protocol(&self) -> Protocol {
        match self {
            Self::Vless { .. } => Protocol::Vless,
            Self::Vmess { .. } => Protocol::Vmess,
            Self::Trojan { .. } => Protocol::Trojan,
            Self::Shadowsocks { .. } => Protocol::Shadowsocks,
            Self::Hysteria2 { .. } => Protocol::Hysteria2,
            Self::Tuic { .. } => Protocol::Tuic,
            Self::Wireguard { .. } => Protocol::Wireguard,
            Self::Socks { .. } => Protocol::Socks,
            Self::Http { .. } => Protocol::Http,
        }
    }
}

/// Traffic / expiry information, as in the `subscription-userinfo` header.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct TrafficInfo {
    /// Bytes uploaded.
    pub upload: u64,
    /// Bytes downloaded.
    pub download: u64,
    /// Quota in bytes; `None` or `Some(0)` = unlimited.
    pub total: Option<u64>,
    /// Expiry as unix seconds; `None` or `Some(0)` = never.
    pub expire: Option<Timestamp>,
}

impl TrafficInfo {
    pub fn used(&self) -> u64 {
        self.upload.saturating_add(self.download)
    }

    /// Remaining bytes, `None` when unlimited.
    pub fn remaining(&self) -> Option<u64> {
        match self.total {
            Some(t) if t > 0 => Some(t.saturating_sub(self.used())),
            _ => None,
        }
    }

    /// Fraction 0.0..=1.0 of the quota used, `None` when unlimited.
    pub fn used_fraction(&self) -> Option<f32> {
        match self.total {
            Some(t) if t > 0 => Some((self.used() as f64 / t as f64).min(1.0) as f32),
            _ => None,
        }
    }
}

/// One proxy server (a parsed share link or a manually created entry).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Server {
    /// Stable id: see [`Server::compute_id`]. Unique within the whole app.
    pub id: String,
    /// Display name (the `#remark` of a share link), may contain a flag emoji.
    pub name: String,
    /// ISO 3166-1 alpha-2 country code in upper case, if known (from a flag emoji / name).
    #[serde(default)]
    pub country: Option<String>,
    pub address: String,
    pub port: u16,
    pub credentials: Credentials,
    #[serde(default)]
    pub transport: Transport,
    #[serde(default)]
    pub security: Security,
    /// The original share link, kept for export / "copy link".
    #[serde(default)]
    pub raw_link: Option<String>,
    /// Per-server quota delivered by the subscription (see `subscription` module docs).
    #[serde(default)]
    pub traffic: Option<TrafficInfo>,
    /// Traffic this client pushed through the server (local accounting, bytes).
    #[serde(default)]
    pub local_traffic: TrafficInfo,
    /// Last latency test result in milliseconds; `Some(None)` = tested, failed.
    #[serde(default)]
    pub last_ping_ms: Option<Option<u32>>,
}

impl Server {
    pub fn protocol(&self) -> Protocol {
        self.credentials.protocol()
    }

    /// "vless / ws / tls" — the subtitle shown under a server in the list.
    pub fn summary(&self) -> String {
        match self.protocol() {
            Protocol::Hysteria2 | Protocol::Tuic | Protocol::Wireguard => {
                self.protocol().as_str().to_string()
            }
            p => format!(
                "{} / {} / {}",
                p.as_str(),
                self.transport.network.as_str(),
                self.security.as_str()
            ),
        }
    }

    /// Stable id derived from the connection parameters (not from the name), so the same
    /// server keeps its id (and its pin/ping/traffic data) across subscription refreshes.
    /// `scope` is the owning subscription id (or "local").
    pub fn compute_id(scope: &str, address: &str, port: u16, credentials: &Credentials) -> String {
        use sha2::{Digest, Sha256};
        let creds = serde_json::to_string(credentials).unwrap_or_default();
        let mut h = Sha256::new();
        h.update(scope.as_bytes());
        h.update([0]);
        h.update(address.as_bytes());
        h.update(port.to_be_bytes());
        h.update(creds.as_bytes());
        hex::encode(&h.finalize()[..8])
    }
}

/// A subscription: a URL that returns a list of servers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Subscription {
    pub id: String,
    /// Name: user given, else `profile-title` header, else host of the URL.
    pub name: String,
    /// Last name set automatically from the subscription (`profile-title` / `content-disposition`).
    /// Lets [`subscription::update`](crate::subscription::update) refresh the name on later
    /// fetches while keeping a name the user changed by hand.
    #[serde(default)]
    pub auto_title: Option<String>,
    /// Primary URL as entered by the user.
    pub url: String,
    /// Backup URLs (from the user and/or from subscription headers), tried in order when the
    /// primary fails. See `subscription` module docs for the header format.
    #[serde(default)]
    pub backup_urls: Vec<String>,
    /// The URL that answered last time (one of `url` / `backup_urls`).
    #[serde(default)]
    pub last_working_url: Option<String>,
    /// Auto-update interval in hours (from `profile-update-interval` or user), 0 = manual.
    #[serde(default)]
    pub update_interval_hours: u32,
    #[serde(default)]
    pub last_update: Option<Timestamp>,
    /// Last update error, shown in the UI.
    #[serde(default)]
    pub last_error: Option<String>,
    /// From `subscription-userinfo`.
    #[serde(default)]
    pub traffic: Option<TrafficInfo>,
    #[serde(default)]
    pub support_url: Option<String>,
    #[serde(default)]
    pub web_page_url: Option<String>,
    /// From `announce` header (provider message), shown above the server list.
    #[serde(default)]
    pub announce: Option<String>,
    /// Custom User-Agent; `None` = default (see `subscription::DEFAULT_USER_AGENT`).
    #[serde(default)]
    pub user_agent: Option<String>,
    /// Pinned subscriptions are shown first.
    #[serde(default)]
    pub pinned: bool,
    /// UI state: collapsed in the server list.
    #[serde(default)]
    pub collapsed: bool,
    #[serde(default)]
    pub servers: Vec<Server>,
    /// Extra data delivered by the provider (Happ / v2RayTun / Remnawave compatible headers).
    #[serde(default)]
    pub provider: ProviderExtras,
    /// User options of this subscription.
    #[serde(default)]
    pub options: SubscriptionOptions,
}

/// Provider-controlled extras of a subscription (see `docs/research/*.md`, `docs/subscription-headers.md`).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ProviderExtras {
    /// Happ `providerid` (URL fragment `#?providerid=`, body `#providerid X`, or header).
    pub provider_id: Option<String>,
    /// Routing profile pushed by the provider (Happ `routing` / `happ://routing/...`,
    /// v2RayTun base64 Xray routing JSON). Applied when connecting through this subscription.
    pub routing: Option<ProviderRouting>,
    /// Announcement link (v2RayTun `announce-url`).
    pub announce_url: Option<String>,
    /// Happ info banner (`sub-info-text`, `sub-info-color`, `sub-info-button-text`, `sub-info-button-link`).
    pub info_banner: Option<InfoBanner>,
    /// Happ `sub-expire` notice text and `sub-expire-button-link`.
    pub expire_notice: Option<String>,
    pub expire_button_link: Option<String>,
    /// HWID device-limit state from response headers (Remnawave `x-hwid-*`).
    pub hwid: Option<HwidStatus>,
    /// v2RayTun `update-always`: refresh on every app start (while disconnected).
    pub update_always: bool,
    /// v2RayTun `network-filter` (only show servers whose transport matches), raw value.
    pub network_filter: Option<String>,
    /// Remnawave `subscription-refill-date` (traffic reset), unix seconds.
    pub refill_date: Option<Timestamp>,
    /// Happ app-management keys (`routing-enable`, `tun-type`, `socks-auth-*`, `http-auth-*`,
    /// `new-url`, `new-domain`, `custom-tunnel-config`, ...): stored raw (lower-case key -> value);
    /// the app decides which ones to honour.
    pub app_overrides: BTreeMap<String, String>,
}

/// A provider routing profile, normalised to Flamix rules.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ProviderRouting {
    pub name: String,
    /// User switch (Happ `routing-enable` / `happ://routing/off` set it to false).
    pub enabled: bool,
    /// Happ `GlobalProxy`: everything not matched goes through the proxy (else direct).
    pub global_proxy: bool,
    /// Rules in order (Happ Block*/Direct*/Proxy* lists, or Xray `rules[]` with outboundTag).
    pub rules: Vec<RoutingRule>,
    pub domain_strategy: Option<String>,
    /// Remote / direct DNS from the profile, if given.
    pub dns: Option<Dns>,
    /// Static hosts (Happ `DnsHosts`).
    pub dns_hosts: BTreeMap<String, String>,
    pub fake_dns: bool,
    /// Custom geo files (Happ `Geoipurl` / `Geositeurl`).
    pub geoip_url: Option<String>,
    pub geosite_url: Option<String>,
    /// Original JSON as received, for display/debug.
    pub raw: Option<serde_json::Value>,
}

/// Happ-style info banner shown above the server list.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct InfoBanner {
    pub text: String,
    /// Colour name or `#RRGGBB`.
    pub color: Option<String>,
    pub button_text: Option<String>,
    pub button_link: Option<String>,
}

/// Device-limit state reported by the panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct HwidStatus {
    pub limit: Option<u32>,
    pub active: Option<u32>,
    pub max_devices_reached: bool,
    pub not_supported: bool,
}

/// User options of a subscription.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SubscriptionOptions {
    /// Extra request headers sent when fetching (v2rayN "custom headers").
    pub custom_headers: BTreeMap<String, String>,
    /// Keep only servers whose name matches this regex (v2rayN "Filter").
    pub name_filter: Option<String>,
    /// Ignore provider routing for this subscription.
    pub ignore_provider_routing: bool,
}

/// How traffic is captured.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ConnectionMode {
    /// Local SOCKS/HTTP inbound + OS system proxy.
    #[default]
    Proxy,
    /// Virtual network interface capturing all traffic (needs admin/root).
    Tun,
}

/// How the core-level balancer spreads new connections across the primary and its backups.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FailoverMode {
    /// Send everything to the lowest-latency healthy node (`leastPing` / `urltest`).
    #[default]
    Failover,
    /// Spread new connections over the healthy nodes (`roundRobin`).
    LoadBalance,
}

/// Core-level failover settings (stage 1, proxy mode only).
///
/// Primary is the selected server; `backups` are ordered server ids. This is the fallback
/// path without the local gate: one core process with an outbound per node and an
/// auto-selecting balancer (Xray `observatory` + `routing.balancers`, sing-box `urltest`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Failover {
    pub enabled: bool,
    pub mode: FailoverMode,
    /// Backup server ids, ordered; the primary is the selected server.
    pub backups: Vec<String>,
    /// Health-probe interval in milliseconds.
    pub probe_interval_ms: u32,
    /// Probe URL; empty means use `Settings::ping_url`.
    pub probe_url: String,
    /// sing-box urltest tolerance in milliseconds.
    pub tolerance_ms: u32,
}

impl Default for Failover {
    fn default() -> Self {
        Self {
            enabled: false,
            mode: FailoverMode::default(),
            backups: Vec::new(),
            probe_interval_ms: 15000,
            probe_url: String::new(),
            tolerance_ms: 50,
        }
    }
}

/// Where a routing rule sends matching traffic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RuleAction {
    Proxy,
    Direct,
    Block,
}

/// One routing rule. Values use Xray syntax ("geosite:category-ru", "domain:example.com",
/// "full:x.y", "regexp:...", "geoip:ru", "10.0.0.0/8", "process:telegram.exe").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoutingRule {
    #[serde(default)]
    pub domains: Vec<String>,
    #[serde(default)]
    pub ips: Vec<String>,
    /// Process names (TUN mode only; sing-box `process_name`).
    #[serde(default)]
    pub processes: Vec<String>,
    /// "tcp", "udp" or empty for both.
    #[serde(default)]
    pub network: Option<String>,
    #[serde(default)]
    pub ports: Option<String>,
    pub action: RuleAction,
    #[serde(default = "yes")]
    pub enabled: bool,
}

fn yes() -> bool {
    true
}

/// Built-in routing presets, like v2rayN / Happ.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RoutingPreset {
    /// Everything through the proxy (private ranges direct).
    Global,
    /// Russian sites/IPs and private ranges direct, the rest proxied; ads blocked.
    #[default]
    BypassRu,
    /// Only `custom_rules`; unmatched traffic goes through the proxy.
    Custom,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Routing {
    pub preset: RoutingPreset,
    /// Applied before the preset rules.
    pub custom_rules: Vec<RoutingRule>,
    /// Xray domainStrategy: "AsIs", "IPIfNonMatch", "IPOnDemand".
    pub domain_strategy: String,
    /// Block QUIC (UDP/443) so browsers fall back to TCP through the proxy.
    pub block_quic: bool,
}

impl Default for Routing {
    fn default() -> Self {
        Self {
            preset: RoutingPreset::default(),
            custom_rules: Vec::new(),
            domain_strategy: "IPIfNonMatch".into(),
            block_quic: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Dns {
    /// Upstream for proxied lookups, e.g. "https://1.1.1.1/dns-query".
    pub remote: String,
    /// Upstream for direct lookups, e.g. "https://77.88.8.8/dns-query" or "localhost".
    pub direct: String,
    /// Fake-IP in TUN mode.
    pub fake_ip: bool,
}

impl Default for Dns {
    fn default() -> Self {
        Self {
            remote: "https://1.1.1.1/dns-query".into(),
            direct: "https://77.88.8.8/dns-query".into(),
            fake_ip: false,
        }
    }
}

/// UI theme.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    #[default]
    System,
    Dark,
    Light,
}

/// User settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub mode: ConnectionMode,
    /// Local mixed (SOCKS5 + HTTP) inbound port.
    pub socks_port: u16,
    pub http_port: u16,
    /// Listen on 0.0.0.0 instead of 127.0.0.1 (share proxy with LAN).
    pub allow_lan: bool,
    /// Set the OS system proxy in Proxy mode.
    pub set_system_proxy: bool,
    /// Hosts that bypass the system proxy.
    pub proxy_bypass: Vec<String>,
    pub routing: Routing,
    pub dns: Dns,
    /// Core-level failover (proxy mode only, stage 1).
    pub failover: Failover,
    /// TUN interface MTU and stack ("system", "gvisor", "mixed").
    pub tun_mtu: u32,
    pub tun_stack: String,
    pub tun_ipv6: bool,
    /// Enable xray `mux` (not for xtls flows).
    pub mux: bool,
    /// Global uTLS fingerprint used when a link has none.
    pub default_fingerprint: String,
    /// URL used for "real" latency tests through the proxy.
    pub ping_url: String,
    /// "tcp" (TCP connect to server) or "url" (HTTP GET through the core).
    pub ping_method: String,
    pub launch_at_login: bool,
    pub connect_on_launch: bool,
    pub minimize_to_tray: bool,
    pub auto_update_subscriptions: bool,
    pub theme: Theme,
    /// UI language: "ru" or "en".
    pub language: String,
    pub log_level: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            mode: ConnectionMode::Proxy,
            socks_port: 10808,
            http_port: 10809,
            allow_lan: false,
            set_system_proxy: true,
            proxy_bypass: vec![
                "localhost".into(),
                "127.*".into(),
                "10.*".into(),
                "172.16.*".into(),
                "192.168.*".into(),
                "<local>".into(),
            ],
            routing: Routing::default(),
            dns: Dns::default(),
            failover: Failover::default(),
            tun_mtu: 9000,
            tun_stack: "mixed".into(),
            tun_ipv6: false,
            mux: false,
            default_fingerprint: "chrome".into(),
            ping_url: "https://www.gstatic.com/generate_204".into(),
            ping_method: "url".into(),
            launch_at_login: false,
            connect_on_launch: false,
            minimize_to_tray: true,
            auto_update_subscriptions: true,
            theme: Theme::System,
            language: "ru".into(),
            log_level: "warning".into(),
        }
    }
}

/// Everything persisted by the app (state.json in the config dir).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AppState {
    /// Schema version for migrations.
    pub version: u32,
    pub subscriptions: Vec<Subscription>,
    /// Servers added manually (links, clipboard, QR), not owned by a subscription.
    pub local_servers: Vec<Server>,
    /// Id of the selected server.
    pub selected_server: Option<String>,
    pub settings: Settings,
    /// Total traffic counters per day ("YYYY-MM-DD" -> info) for the statistics page.
    pub daily_traffic: BTreeMap<String, TrafficInfo>,
    /// Random stable device id (hex), generated by the store on first load; sent as `x-hwid`.
    pub device_id: Option<String>,
}

impl AppState {
    pub const VERSION: u32 = 1;

    /// Iterate over every server (subscriptions first, then local ones).
    pub fn servers(&self) -> impl Iterator<Item = &Server> {
        self.subscriptions
            .iter()
            .flat_map(|s| s.servers.iter())
            .chain(self.local_servers.iter())
    }

    pub fn server(&self, id: &str) -> Option<&Server> {
        self.servers().find(|s| s.id == id)
    }

    pub fn server_mut(&mut self, id: &str) -> Option<&mut Server> {
        self.subscriptions
            .iter_mut()
            .flat_map(|s| s.servers.iter_mut())
            .chain(self.local_servers.iter_mut())
            .find(|s| s.id == id)
    }

    pub fn selected(&self) -> Option<&Server> {
        self.selected_server
            .as_deref()
            .and_then(|id| self.server(id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn traffic_math() {
        let t = TrafficInfo {
            upload: 5,
            download: 10,
            total: Some(100),
            expire: None,
        };
        assert_eq!(t.used(), 15);
        assert_eq!(t.remaining(), Some(85));
        assert!((t.used_fraction().unwrap() - 0.15).abs() < 1e-6);
        let unlimited = TrafficInfo {
            total: Some(0),
            ..t
        };
        assert_eq!(unlimited.remaining(), None);
    }

    #[test]
    fn state_roundtrip_and_defaults() {
        let s: AppState = serde_json::from_str("{}").unwrap();
        assert_eq!(s.settings.socks_port, 10808);
        let json = serde_json::to_string(&s).unwrap();
        let back: AppState = serde_json::from_str(&json).unwrap();
        assert_eq!(s, back);
    }

    #[test]
    fn failover_defaults_and_serde() {
        let f = Failover::default();
        assert!(!f.enabled);
        assert_eq!(f.mode, FailoverMode::Failover);
        assert!(f.backups.is_empty());
        assert_eq!(f.probe_interval_ms, 15000);
        assert!(f.probe_url.is_empty());
        assert_eq!(f.tolerance_ms, 50);
        assert_eq!(Settings::default().failover, f);

        // Old state files (no `failover` key) keep loading with the defaults.
        let s: Settings = serde_json::from_str("{}").unwrap();
        assert_eq!(s.failover, f);
        let json = serde_json::to_string(&f).unwrap();
        assert_eq!(serde_json::from_str::<Failover>(&json).unwrap(), f);
    }

    #[test]
    fn server_id_ignores_name() {
        let c = Credentials::Trojan {
            password: "p".into(),
        };
        let a = Server::compute_id("sub1", "h", 443, &c);
        assert_eq!(a, Server::compute_id("sub1", "h", 443, &c));
        assert_ne!(a, Server::compute_id("sub2", "h", 443, &c));
        assert_eq!(a.len(), 16);
    }
}

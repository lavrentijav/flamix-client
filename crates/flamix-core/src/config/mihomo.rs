//! mihomo (Clash.Meta) config generator.
//!
//! One YAML file per connection: a single proxy (the selected server), the `PROXY` / `AUTO` /
//! `DISCORD` / `ANIXART-MIRROR` groups, a `mixed` listener (Proxy mode) or a `tun` section (TUN
//! mode), DNS, and rules. The RF preset is rendered 1:1 from `data/ru-routing.yaml`
//! ([`crate::config::ru_routing`]); Global and Custom work too.
//!
//! The config is built as a [`serde_json::Value`] (the type of `CorePlan::config`) and written
//! as YAML with [`render_yaml`].

use crate::config::ru_routing::{self, RuAction, RuleKind};
use crate::config::{effective_local_auth, ConnectContext};
use crate::model::{
    ConnectionMode, Credentials, MirrorMode, Network, Routing, RoutingPreset, RuleAction, Security,
    Server, Settings,
};
use serde_json::{json, Map, Value};

/// `country.mmdb` fallback (mihomo only reads it when `geodata-mode` is off, but the key is
/// part of the documented `geox-url`).
const MMDB_URL: &str =
    "https://github.com/MetaCubeX/meta-rules-dat/releases/download/latest/country.mmdb";

/// geosite tags of the runetfreedom database that mihomo's loader rejects (the list holds
/// domains with a trailing dot: `invalid domain "+.x.ru.": trailing dot is not allowed`,
/// which aborts the whole config). They are left out of the mihomo rules; `ru-blocked` and
/// the remote lists cover the same services.
const MIHOMO_BAD_GEOSITE: [&str; 1] = ["ru-blocked-all"];

const RESERVED_NAMES: [&str; 6] = [
    "PROXY",
    "AUTO",
    "DISCORD",
    "ANIXART-MIRROR",
    "DIRECT",
    "REJECT",
];

/// Serialize a generated config to the YAML mihomo reads (`mihomo -f`).
pub fn render_yaml(config: &Value) -> Result<String, String> {
    serde_yaml::to_string(config).map_err(|e| e.to_string())
}

/// Full mihomo config; `None` when mihomo can't express `server`.
pub fn full_config(server: &Server, settings: &Settings, api_port: u16) -> Option<Value> {
    full_config_with(server, settings, api_port, &ConnectContext::default())
}

/// Like [`full_config`], with the Happ local-auth overrides of a [`ConnectContext`].
/// Provider routing profiles are not supported by this core and are ignored.
pub fn full_config_with(
    server: &Server,
    settings: &Settings,
    api_port: u16,
    ctx: &ConnectContext,
) -> Option<Value> {
    let name = proxy_name(server);
    let proxy = proxy(server, settings, &name)?;
    if ctx.provider.is_some_and(|p| p.enabled) {
        tracing::warn!("provider routing profiles are not supported by mihomo; ignored");
    }
    let tun = settings.mode == ConnectionMode::Tun;
    let routing = &settings.routing;

    let mut cfg = Map::new();
    cfg.insert("mixed-port".into(), json!(settings.socks_port));
    cfg.insert("allow-lan".into(), json!(settings.allow_lan));
    cfg.insert("mode".into(), json!("rule"));
    cfg.insert("log-level".into(), json!(log_level(&settings.log_level)));
    cfg.insert("ipv6".into(), json!(false));
    cfg.insert("find-process-mode".into(), json!("strict"));
    cfg.insert(
        "external-controller".into(),
        json!(format!("127.0.0.1:{api_port}")),
    );
    if let Some(auth) = effective_local_auth("socks", settings, ctx.overrides) {
        cfg.insert(
            "authentication".into(),
            json!([format!("{}:{}", auth.user, auth.password)]),
        );
    }
    cfg.insert("geodata-mode".into(), json!(true));
    cfg.insert("geo-auto-update".into(), json!(true));
    cfg.insert("geo-update-interval".into(), json!(24));
    cfg.insert("geox-url".into(), geox_url(settings));

    if tun {
        cfg.insert(
            "tun".into(),
            json!({
                "enable": true,
                "stack": "mixed",
                "auto-route": true,
                "auto-detect-interface": true,
                "dns-hijack": ["any:53", "tcp://any:53"],
                "strict-route": true,
            }),
        );
    } else {
        cfg.insert("tun".into(), json!({"enable": false}));
    }
    cfg.insert("dns".into(), dns(settings));
    cfg.insert("proxies".into(), json!([proxy]));
    cfg.insert("proxy-groups".into(), groups(&name, settings));

    let rf = routing.preset == RoutingPreset::BypassRu;
    if rf {
        cfg.insert("rule-providers".into(), rule_providers());
    }
    cfg.insert("rules".into(), json!(rules(routing)));
    Some(Value::Object(cfg))
}

/// True when mihomo can run `server`.
pub fn supports(server: &Server, settings: &Settings) -> bool {
    proxy(server, settings, "probe").is_some()
}

fn proxy_name(server: &Server) -> String {
    let name = server.name.trim();
    if name.is_empty() || RESERVED_NAMES.iter().any(|r| r.eq_ignore_ascii_case(name)) {
        "server".to_string()
    } else {
        name.to_string()
    }
}

fn non_empty(value: &Option<String>) -> Option<&str> {
    value.as_deref().filter(|s| !s.is_empty())
}

fn log_level(level: &str) -> String {
    match level.to_ascii_lowercase().as_str() {
        "warn" | "warning" => "warning".into(),
        "none" | "silent" => "silent".into(),
        "error" => "error".into(),
        "info" => "info".into(),
        "debug" | "trace" => "debug".into(),
        _ => "warning".into(),
    }
}

fn geox_url(settings: &Settings) -> Value {
    let urls = crate::geo::urls(&settings.geo);
    let mut m = Map::new();
    if !urls.xray_geoip.is_empty() {
        m.insert("geoip".into(), json!(urls.xray_geoip));
    }
    if !urls.xray_geosite.is_empty() {
        m.insert("geosite".into(), json!(urls.xray_geosite));
    }
    m.insert("mmdb".into(), json!(MMDB_URL));
    Value::Object(m)
}

/// TLS-related proxy keys shared by vless/vmess/trojan. `None` = unsupported combination.
fn add_tls(obj: &mut Map<String, Value>, server: &Server, settings: &Settings, trojan: bool) {
    let fp = |link: &Option<String>| -> String {
        non_empty(link)
            .map(str::to_string)
            .unwrap_or_else(|| settings.default_fingerprint.clone())
    };
    match &server.security {
        Security::None => {
            if trojan {
                // mihomo trojan is always TLS.
                obj.insert("skip-cert-verify".into(), json!(false));
            }
        }
        Security::Tls {
            sni,
            alpn,
            fingerprint,
            allow_insecure,
        } => {
            if !trojan {
                obj.insert("tls".into(), json!(true));
            }
            if let Some(sni) = non_empty(sni) {
                let key = if trojan { "sni" } else { "servername" };
                obj.insert(key.into(), json!(sni));
            }
            if !alpn.is_empty() {
                obj.insert("alpn".into(), json!(alpn));
            }
            let fingerprint = fp(fingerprint);
            if !fingerprint.is_empty() {
                obj.insert("client-fingerprint".into(), json!(fingerprint));
            }
            obj.insert("skip-cert-verify".into(), json!(*allow_insecure));
        }
        Security::Reality {
            sni,
            public_key,
            short_id,
            fingerprint,
            ..
        } => {
            obj.insert("tls".into(), json!(true));
            if let Some(sni) = non_empty(sni) {
                obj.insert("servername".into(), json!(sni));
            }
            let mut reality = Map::new();
            reality.insert("public-key".into(), json!(public_key));
            reality.insert(
                "short-id".into(),
                json!(short_id.clone().unwrap_or_default()),
            );
            obj.insert("reality-opts".into(), Value::Object(reality));
            let fingerprint = fp(fingerprint);
            obj.insert(
                "client-fingerprint".into(),
                json!(if fingerprint.is_empty() {
                    "chrome".to_string()
                } else {
                    fingerprint
                }),
            );
        }
    }
}

/// `network` + `*-opts`; `false` = the transport is not expressible.
fn add_transport(obj: &mut Map<String, Value>, server: &Server) -> bool {
    let t = &server.transport;
    match t.network {
        Network::Tcp => {
            // Fake HTTP headers on raw TCP are not supported.
            if non_empty(&t.header_type).is_some_and(|h| h != "none") {
                return false;
            }
            obj.insert("network".into(), json!("tcp"));
        }
        Network::Ws => {
            obj.insert("network".into(), json!("ws"));
            let mut ws = Map::new();
            if let Some(path) = non_empty(&t.path) {
                ws.insert("path".into(), json!(path));
            }
            if let Some(host) = non_empty(&t.host) {
                ws.insert("headers".into(), json!({"Host": host}));
            }
            if !ws.is_empty() {
                obj.insert("ws-opts".into(), Value::Object(ws));
            }
        }
        Network::Grpc => {
            obj.insert("network".into(), json!("grpc"));
            if let Some(service) = non_empty(&t.service_name) {
                obj.insert("grpc-opts".into(), json!({"grpc-service-name": service}));
            }
        }
        Network::HttpUpgrade | Network::H2 | Network::Xhttp | Network::Kcp | Network::Quic => {
            return false;
        }
    }
    true
}

/// The single `proxies` entry for `server`; `None` when mihomo can't express it.
fn proxy(server: &Server, settings: &Settings, name: &str) -> Option<Value> {
    let mut obj = Map::new();
    obj.insert("name".into(), json!(name));
    obj.insert("server".into(), json!(server.address));
    obj.insert("port".into(), json!(server.port));
    obj.insert("udp".into(), json!(true));
    match &server.credentials {
        Credentials::Vless { uuid, flow, .. } => {
            obj.insert("type".into(), json!("vless"));
            obj.insert("uuid".into(), json!(uuid));
            if let Some(flow) = non_empty(flow) {
                obj.insert("flow".into(), json!(flow));
            }
            add_tls(&mut obj, server, settings, false);
            if !add_transport(&mut obj, server) {
                return None;
            }
        }
        Credentials::Vmess {
            uuid,
            alter_id,
            cipher,
        } => {
            obj.insert("type".into(), json!("vmess"));
            obj.insert("uuid".into(), json!(uuid));
            obj.insert("alterId".into(), json!(alter_id));
            obj.insert(
                "cipher".into(),
                json!(cipher.clone().unwrap_or_else(|| "auto".into())),
            );
            add_tls(&mut obj, server, settings, false);
            if !add_transport(&mut obj, server) {
                return None;
            }
        }
        Credentials::Trojan { password } => {
            obj.insert("type".into(), json!("trojan"));
            obj.insert("password".into(), json!(password));
            add_tls(&mut obj, server, settings, true);
            if !add_transport(&mut obj, server) {
                return None;
            }
        }
        Credentials::Shadowsocks {
            method,
            password,
            plugin,
            plugin_opts,
        } => {
            obj.insert("type".into(), json!("ss"));
            obj.insert("cipher".into(), json!(method));
            obj.insert("password".into(), json!(password));
            if let Some(plugin) = non_empty(plugin) {
                if !matches!(plugin, "obfs-local" | "obfs") {
                    return None;
                }
                let mut opts = Map::new();
                for part in plugin_opts.as_deref().unwrap_or("").split(';') {
                    match part.split_once('=') {
                        Some(("obfs", mode)) => {
                            opts.insert("mode".into(), json!(mode));
                        }
                        Some(("obfs-host", host)) => {
                            opts.insert("host".into(), json!(host));
                        }
                        _ => {}
                    }
                }
                if !opts.contains_key("mode") {
                    return None;
                }
                obj.insert("plugin".into(), json!("obfs"));
                obj.insert("plugin-opts".into(), Value::Object(opts));
            }
            if server.transport.network != Network::Tcp {
                return None;
            }
        }
        Credentials::Hysteria2 {
            password,
            obfs,
            obfs_password,
            ports,
            up_mbps,
            down_mbps,
        } => {
            obj.insert("type".into(), json!("hysteria2"));
            obj.insert("password".into(), json!(password));
            if let Some(ports) = non_empty(ports) {
                obj.insert("ports".into(), json!(ports));
            }
            if let Some(obfs) = non_empty(obfs) {
                obj.insert("obfs".into(), json!(obfs));
                if let Some(password) = non_empty(obfs_password) {
                    obj.insert("obfs-password".into(), json!(password));
                }
            }
            if let Some(up) = up_mbps {
                obj.insert("up".into(), json!(format!("{up} Mbps")));
            }
            if let Some(down) = down_mbps {
                obj.insert("down".into(), json!(format!("{down} Mbps")));
            }
            add_quic_tls(&mut obj, &server.security);
        }
        Credentials::Tuic {
            uuid,
            password,
            congestion_control,
            udp_relay_mode,
        } => {
            obj.insert("type".into(), json!("tuic"));
            obj.insert("uuid".into(), json!(uuid));
            obj.insert("password".into(), json!(password));
            if let Some(cc) = non_empty(congestion_control) {
                obj.insert("congestion-controller".into(), json!(cc));
            }
            if let Some(mode) = non_empty(udp_relay_mode) {
                obj.insert("udp-relay-mode".into(), json!(mode));
            }
            add_quic_tls(&mut obj, &server.security);
        }
        Credentials::Socks { username, password } => {
            obj.insert("type".into(), json!("socks5"));
            add_basic_auth(&mut obj, username, password);
        }
        Credentials::Http { username, password } => {
            obj.insert("type".into(), json!("http"));
            add_basic_auth(&mut obj, username, password);
        }
        Credentials::Wireguard { .. } => return None,
    }
    Some(Value::Object(obj))
}

fn add_basic_auth(
    obj: &mut Map<String, Value>,
    username: &Option<String>,
    password: &Option<String>,
) {
    if let Some(username) = non_empty(username) {
        obj.insert("username".into(), json!(username));
    }
    if let Some(password) = non_empty(password) {
        obj.insert("password".into(), json!(password));
    }
}

/// hysteria2 / tuic carry their TLS parameters at the top level of the proxy.
fn add_quic_tls(obj: &mut Map<String, Value>, security: &Security) {
    if let Security::Tls {
        sni,
        alpn,
        allow_insecure,
        ..
    } = security
    {
        if let Some(sni) = non_empty(sni) {
            obj.insert("sni".into(), json!(sni));
        }
        if !alpn.is_empty() {
            obj.insert("alpn".into(), json!(alpn));
        }
        obj.insert("skip-cert-verify".into(), json!(*allow_insecure));
    }
}

fn groups(name: &str, settings: &Settings) -> Value {
    let mirror = match settings.routing.anixart_mirror {
        MirrorMode::Direct => json!(["DIRECT", "PROXY"]),
        MirrorMode::Proxy => json!(["PROXY", "DIRECT"]),
    };
    let ping = settings.ping_url.trim();
    let url = if ping.is_empty() {
        crate::config::DEFAULT_PROBE_URL
    } else {
        ping
    };
    json!([
        {"name": "PROXY", "type": "select", "proxies": [name, "DIRECT"]},
        {"name": "AUTO", "type": "url-test", "proxies": [name], "url": url, "interval": 300},
        {"name": "DISCORD", "type": "select", "proxies": ["PROXY", "AUTO"]},
        {"name": "ANIXART-MIRROR", "type": "select", "proxies": mirror},
    ])
}

fn dns_server(value: &str, fallback: &str) -> String {
    let value = value.trim();
    match value {
        "" => fallback.to_string(),
        "localhost" | "local" => "system".to_string(),
        other => other.to_string(),
    }
}

fn dns(settings: &Settings) -> Value {
    let remote = dns_server(&settings.dns.remote, "https://1.1.1.1/dns-query");
    let direct = dns_server(&settings.dns.direct, "https://77.88.8.8/dns-query");
    let bootstrap = settings.dns.bootstrap.trim();
    let bootstrap = if bootstrap.is_empty() {
        "77.88.8.8"
    } else {
        bootstrap
    };
    let mut dns = Map::new();
    dns.insert("enable".into(), json!(true));
    dns.insert("ipv6".into(), json!(false));
    dns.insert("enhanced-mode".into(), json!("fake-ip"));
    dns.insert("fake-ip-range".into(), json!("198.18.0.1/16"));
    dns.insert("default-nameserver".into(), json!([bootstrap]));
    dns.insert("nameserver".into(), json!([format!("{remote}#PROXY")]));
    dns.insert("proxy-server-nameserver".into(), json!([direct.clone()]));
    if settings.routing.preset == RoutingPreset::BypassRu {
        let mut policy = Map::new();
        for rule in &ru_routing::data().rules {
            if rule.action != RuAction::Direct || rule.group.is_some() {
                continue;
            }
            match rule.kind {
                RuleKind::DomainSuffix => {
                    policy.insert(format!("+.{}", rule.value), json!(direct.clone()));
                }
                RuleKind::Geosite => {
                    policy.insert(format!("geosite:{}", rule.value), json!(direct.clone()));
                }
                _ => {}
            }
        }
        if !policy.is_empty() {
            dns.insert("nameserver-policy".into(), Value::Object(policy));
        }
    }
    Value::Object(dns)
}

fn rule_providers() -> Value {
    let mut m = Map::new();
    for (name, p) in &ru_routing::data().rule_providers {
        m.insert(
            name.clone(),
            json!({
                "type": "http",
                "url": p.url,
                "behavior": p.behavior,
                "format": p.format,
                "interval": p.interval,
                "path": format!("./rule-providers/{name}.txt"),
            }),
        );
    }
    Value::Object(m)
}

fn target(action: RuleAction) -> &'static str {
    match action {
        RuleAction::Proxy => "PROXY",
        RuleAction::Direct => "DIRECT",
        RuleAction::Block => "REJECT",
    }
}

/// Rule lines of a custom rule (Xray-syntax values). Entries of one kind are alternatives;
/// `network` and `ports` narrow every entry (`AND`).
fn custom_rule_lines(rule: &crate::model::RoutingRule) -> Vec<String> {
    // (matcher, needs no-resolve)
    let mut entries: Vec<(String, bool)> = Vec::new();
    for d in &rule.domains {
        let entry = if let Some(v) = d.strip_prefix("domain:") {
            format!("DOMAIN-SUFFIX,{v}")
        } else if let Some(v) = d.strip_prefix("full:") {
            format!("DOMAIN,{v}")
        } else if let Some(v) = d.strip_prefix("keyword:") {
            format!("DOMAIN-KEYWORD,{v}")
        } else if let Some(v) = d.strip_prefix("regexp:") {
            format!("DOMAIN-REGEX,{v}")
        } else if let Some(v) = d.strip_prefix("geosite:") {
            format!("GEOSITE,{v}")
        } else {
            format!("DOMAIN-SUFFIX,{d}")
        };
        entries.push((entry, false));
    }
    for ip in &rule.ips {
        let entry = if let Some(v) = ip.strip_prefix("geoip:") {
            format!("GEOIP,{}", v.to_uppercase())
        } else {
            format!("IP-CIDR,{ip}")
        };
        entries.push((entry, true));
    }
    for p in &rule.processes {
        entries.push((format!("PROCESS-NAME,{p}"), false));
    }
    let mut narrow: Vec<String> = Vec::new();
    match non_empty(&rule.network) {
        Some("tcp") => narrow.push("(NETWORK,TCP)".into()),
        Some("udp") => narrow.push("(NETWORK,UDP)".into()),
        _ => {}
    }
    if let Some(ports) = non_empty(&rule.ports) {
        let items: Vec<String> = ports
            .split(',')
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .map(|p| format!("(DST-PORT,{p})"))
            .collect();
        match items.len() {
            0 => {}
            1 => narrow.push(items[0].clone()),
            _ => narrow.push(format!("(OR,({}))", items.join(","))),
        }
    }
    let to = target(rule.action);
    if entries.is_empty() {
        if narrow.is_empty() {
            return Vec::new();
        }
        return vec![and_line(&narrow, to)];
    }
    entries
        .into_iter()
        .map(|(entry, no_resolve)| {
            if narrow.is_empty() {
                let nr = if no_resolve { ",no-resolve" } else { "" };
                format!("{entry},{to}{nr}")
            } else {
                // Inside `AND` the `no-resolve` flag stays within the condition.
                let nr = if no_resolve { ",no-resolve" } else { "" };
                let mut all = vec![format!("({entry}{nr})")];
                all.extend(narrow.iter().cloned());
                and_line(&all, to)
            }
        })
        .collect()
}

fn and_line(conds: &[String], to: &str) -> String {
    if conds.len() == 1 {
        let inner = conds[0].trim_start_matches('(').trim_end_matches(')');
        return format!("{inner},{to}");
    }
    format!("AND,({}),{to}", conds.join(","))
}

fn rules(routing: &Routing) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for rule in routing.custom_rules.iter().filter(|r| r.enabled) {
        out.extend(custom_rule_lines(rule));
    }
    if routing.block_quic {
        out.push("AND,((NETWORK,UDP),(DST-PORT,443)),REJECT".into());
    }
    match routing.preset {
        RoutingPreset::Global => {
            out.push("GEOIP,PRIVATE,DIRECT,no-resolve".into());
            out.push("MATCH,PROXY".into());
        }
        RoutingPreset::Custom => out.push("MATCH,PROXY".into()),
        RoutingPreset::BypassRu => {
            for rule in &ru_routing::data().rules {
                if rule.kind == RuleKind::Geosite
                    && MIHOMO_BAD_GEOSITE.contains(&rule.value.as_str())
                {
                    tracing::warn!(
                        "mihomo: geosite '{}' skipped (rejected by its loader)",
                        rule.value
                    );
                    continue;
                }
                let to = match (&rule.group, rule.action) {
                    (Some(group), _) => group.as_str(),
                    (None, RuAction::Direct) => "DIRECT",
                    (None, RuAction::Proxy) => "PROXY",
                    (None, RuAction::Mirror) => "ANIXART-MIRROR",
                };
                let v = &rule.value;
                out.push(match rule.kind {
                    RuleKind::DomainSuffix => format!("DOMAIN-SUFFIX,{v},{to}"),
                    RuleKind::DomainKeyword => format!("DOMAIN-KEYWORD,{v},{to}"),
                    RuleKind::Geosite => format!("GEOSITE,{v},{to}"),
                    RuleKind::Geoip => format!("GEOIP,{v},{to},no-resolve"),
                    RuleKind::IpCidr => format!("IP-CIDR,{v},{to},no-resolve"),
                    RuleKind::DstPort => format!("DST-PORT,{v},{to}"),
                    RuleKind::ProcessRegex => format!("PROCESS-NAME-REGEX,{v},{to}"),
                    RuleKind::RuleSet => format!("RULE-SET,{v},{to},no-resolve"),
                    RuleKind::Final => "MATCH,DIRECT".to_string(),
                });
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{RoutingRule, Transport};

    fn server(credentials: Credentials, transport: Transport, security: Security) -> Server {
        Server {
            id: "t".into(),
            name: "node".into(),
            country: None,
            address: "example.com".into(),
            port: 443,
            credentials,
            transport,
            security,
            raw_link: None,
            traffic: None,
            local_traffic: Default::default(),
            last_ping_ms: None,
        }
    }

    fn vless(transport: Transport, security: Security) -> Server {
        server(
            Credentials::Vless {
                uuid: "11111111-1111-1111-1111-111111111111".into(),
                flow: None,
                encryption: None,
            },
            transport,
            security,
        )
    }

    #[test]
    fn unsupported_transports_are_none() {
        for network in [Network::Xhttp, Network::Kcp, Network::Quic, Network::H2] {
            let s = vless(
                Transport {
                    network,
                    ..Transport::default()
                },
                Security::None,
            );
            assert!(
                full_config(&s, &Settings::default(), 1).is_none(),
                "{network:?}"
            );
        }
    }

    #[test]
    fn rf_rules_are_one_to_one() {
        let cfg = full_config(
            &vless(Transport::default(), Security::None),
            &Settings::default(),
            10820,
        )
        .unwrap();
        let rules = cfg["rules"].as_array().unwrap();
        // 391 data rules minus the geosite tag mihomo's loader rejects.
        assert_eq!(rules.len(), 390);
        assert_eq!(rules[0], json!("DST-PORT,53,DIRECT"));
        assert_eq!(rules.last().unwrap(), &json!("MATCH,DIRECT"));
        assert!(rules.contains(&json!("RULE-SET,itdog-discord-ip,DISCORD,no-resolve")));
        assert!(rules.contains(&json!("DOMAIN-SUFFIX,anixart.tv,ANIXART-MIRROR")));
        assert!(rules.contains(&json!("GEOSITE,discord,DISCORD")));
        assert!(rules
            .iter()
            .any(|r| r.as_str().unwrap().starts_with("PROCESS-NAME-REGEX,")));
        assert_eq!(cfg["rule-providers"].as_object().unwrap().len(), 3);
        assert_eq!(cfg["tun"]["enable"], json!(false));
        assert_eq!(cfg["mixed-port"], json!(10808));
    }

    #[test]
    fn mirror_initial_value_and_tun() {
        let mut settings = Settings {
            mode: ConnectionMode::Tun,
            ..Settings::default()
        };
        settings.routing.anixart_mirror = MirrorMode::Proxy;
        let cfg = full_config(&vless(Transport::default(), Security::None), &settings, 1).unwrap();
        assert_eq!(cfg["proxy-groups"][3]["proxies"][0], json!("PROXY"));
        assert_eq!(cfg["tun"]["enable"], json!(true));
        assert_eq!(cfg["tun"]["strict-route"], json!(true));
        assert_eq!(cfg["dns"]["fake-ip-range"], json!("198.18.0.1/16"));
    }

    #[test]
    fn global_and_custom_presets() {
        let s = vless(Transport::default(), Security::None);
        let mut settings = Settings::default();
        settings.routing.preset = RoutingPreset::Global;
        let cfg = full_config(&s, &settings, 1).unwrap();
        assert_eq!(
            cfg["rules"],
            json!(["GEOIP,PRIVATE,DIRECT,no-resolve", "MATCH,PROXY"])
        );
        assert!(cfg.get("rule-providers").is_none());

        settings.routing.preset = RoutingPreset::Custom;
        settings.routing.custom_rules = vec![RoutingRule {
            domains: vec!["domain:a.example".into(), "keyword:ads".into()],
            ips: vec!["1.2.3.0/24".into()],
            processes: vec!["tg.exe".into()],
            network: None,
            ports: None,
            action: RuleAction::Block,
            enabled: true,
        }];
        let cfg = full_config(&s, &settings, 1).unwrap();
        assert_eq!(
            cfg["rules"],
            json!([
                "DOMAIN-SUFFIX,a.example,REJECT",
                "DOMAIN-KEYWORD,ads,REJECT",
                "IP-CIDR,1.2.3.0/24,REJECT,no-resolve",
                "PROCESS-NAME,tg.exe,REJECT",
                "MATCH,PROXY"
            ])
        );
    }

    #[test]
    fn yaml_renders() {
        let cfg = full_config(
            &vless(Transport::default(), Security::None),
            &Settings::default(),
            1,
        )
        .unwrap();
        let yaml = render_yaml(&cfg).unwrap();
        assert!(yaml.contains("mixed-port: 10808"));
        let back: serde_yaml::Value = serde_yaml::from_str(&yaml).unwrap();
        assert!(back.get("rules").is_some());
    }
}

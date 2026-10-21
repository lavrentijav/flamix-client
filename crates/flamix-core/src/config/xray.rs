//! Xray-core JSON config. OWNER: task T03.
//!
//! Targets current Xray-core (v25/v26): the `raw` network replaces `tcp` (with
//! `rawSettings.header`), `xhttpSettings` carries the xhttp mode/extra, and the
//! DNS `servers` array takes `{address, domains}` objects.

use crate::config::routing::{self, Rule};
use crate::config::{effective_local_auth, supported_by, ConnectContext, CoreKind};
use crate::model::{
    ConnectionMode, Credentials, FailoverMode, Network, ProviderRouting, RuleAction, Security,
    Server, Settings, Transport,
};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;

/// Outbound object (`"tag": "proxy"`) for `server`.
pub fn outbound(server: &Server, settings: &Settings) -> Value {
    outbound_with_tag(server, settings, "proxy")
}

/// Outbound object for `server` with an explicit `tag` (failover nodes use `node-N`).
fn outbound_with_tag(server: &Server, settings: &Settings, tag: &str) -> Value {
    let mut obj = Map::new();
    obj.insert("tag".into(), json!(tag));

    match &server.credentials {
        Credentials::Vless {
            uuid,
            flow,
            encryption,
        } => {
            obj.insert("protocol".into(), json!("vless"));
            let mut user = Map::new();
            user.insert("id".into(), json!(uuid));
            user.insert(
                "encryption".into(),
                json!(encryption.clone().unwrap_or_else(|| "none".into())),
            );
            if let Some(flow) = non_empty(flow) {
                user.insert("flow".into(), json!(flow));
            }
            obj.insert(
                "settings".into(),
                json!({"vnext": [{
                    "address": &server.address,
                    "port": server.port,
                    "users": [Value::Object(user)],
                }]}),
            );
            add_stream(&mut obj, &server.transport, &server.security, settings);
            add_mux(&mut obj, settings, non_empty(flow).is_none());
        }
        Credentials::Vmess {
            uuid,
            alter_id,
            cipher,
        } => {
            obj.insert("protocol".into(), json!("vmess"));
            let user = json!({
                "id": uuid,
                "alterId": alter_id,
                "security": cipher.clone().unwrap_or_else(|| "auto".into()),
            });
            obj.insert(
                "settings".into(),
                json!({"vnext": [{
                    "address": &server.address,
                    "port": server.port,
                    "users": [user],
                }]}),
            );
            add_stream(&mut obj, &server.transport, &server.security, settings);
            add_mux(&mut obj, settings, true);
        }
        Credentials::Trojan { password } => {
            obj.insert("protocol".into(), json!("trojan"));
            obj.insert(
                "settings".into(),
                json!({"servers": [{
                    "address": &server.address,
                    "port": server.port,
                    "password": password,
                }]}),
            );
            add_stream(&mut obj, &server.transport, &server.security, settings);
            add_mux(&mut obj, settings, true);
        }
        Credentials::Shadowsocks {
            method, password, ..
        } => {
            obj.insert("protocol".into(), json!("shadowsocks"));
            // SIP003 plugins are unsupported by Xray; the outbound is emitted as-is.
            obj.insert(
                "settings".into(),
                json!({"servers": [{
                    "address": &server.address,
                    "port": server.port,
                    "method": method,
                    "password": password,
                }]}),
            );
            add_mux(&mut obj, settings, true);
        }
        Credentials::Socks { username, password } => {
            obj.insert("protocol".into(), json!("socks"));
            obj.insert(
                "settings".into(),
                server_settings(server, username, password),
            );
        }
        Credentials::Http { username, password } => {
            obj.insert("protocol".into(), json!("http"));
            obj.insert(
                "settings".into(),
                server_settings(server, username, password),
            );
        }
        Credentials::Wireguard {
            private_key,
            peer_public_key,
            pre_shared_key,
            local_address,
            reserved,
            mtu,
        } => {
            obj.insert("protocol".into(), json!("wireguard"));
            let mut wg = Map::new();
            wg.insert("secretKey".into(), json!(private_key));
            if !local_address.is_empty() {
                wg.insert("address".into(), json!(local_address));
            }
            let mut peer = Map::new();
            peer.insert("publicKey".into(), json!(peer_public_key));
            if let Some(psk) = non_empty(pre_shared_key) {
                peer.insert("preSharedKey".into(), json!(psk));
            }
            peer.insert(
                "endpoint".into(),
                json!(format!("{}:{}", server.address, server.port)),
            );
            peer.insert("allowedIPs".into(), json!(["0.0.0.0/0", "::/0"]));
            wg.insert("peers".into(), json!([Value::Object(peer)]));
            if let Some(reserved) = reserved {
                wg.insert("reserved".into(), json!(reserved));
            }
            if let Some(mtu) = mtu {
                wg.insert("mtu".into(), json!(mtu));
            }
            obj.insert("settings".into(), Value::Object(wg));
        }
        // sing-box-only protocols never reach the Xray generator.
        Credentials::Hysteria2 { .. } | Credentials::Tuic { .. } => {
            obj.insert("protocol".into(), json!("freedom"));
        }
    }

    Value::Object(obj)
}

/// Full config: log, dns, inbounds (socks + http on `settings` ports, plus `api` inbound
/// for stats on 127.0.0.1:`api_port`), outbounds (proxy, direct, block), routing from
/// `settings.routing`, `stats` + `policy` for traffic counters.
pub fn full_config(server: &Server, settings: &Settings, api_port: u16) -> Value {
    full_config_with(server, settings, api_port, &ConnectContext::default())
}

/// Like [`full_config`], but with the selected subscription's provider routing and Happ
/// app-overrides ([`ConnectContext`]). See [`crate::config::plan_with`].
pub fn full_config_with(
    server: &Server,
    settings: &Settings,
    api_port: u16,
    ctx: &ConnectContext,
) -> Value {
    full_config_cluster_with(std::slice::from_ref(server), settings, api_port, ctx)
}

/// Full config for a primary node plus ordered backups (`node-0` is the primary).
///
/// With one proxy-capable node this is exactly [`full_config_with`]. With
/// `settings.failover.enabled`, Proxy mode, and more than one node, every node becomes an
/// outbound tagged `node-N`, `direct`/`block` stay, and a balancer named `proxy` is added
/// under `routing.balancers` (`leastPing` for [`FailoverMode::Failover`], `roundRobin` for
/// [`FailoverMode::LoadBalance`]) together with a top-level `observatory` probing the nodes.
/// The routing generator is unchanged: its rules keep naming the tag `proxy`. Provider
/// routing/overrides belong to the primary node and are applied to the whole config.
///
/// Defensive: nodes Xray cannot proxy (hysteria2/tuic) are skipped, so a cluster never
/// contains a `freedom`/direct node (see [`crate::config::plan_cluster`], which also drops
/// them with a warning).
pub(crate) fn full_config_cluster_with(
    nodes: &[Server],
    settings: &Settings,
    api_port: u16,
    ctx: &ConnectContext,
) -> Value {
    let supported: Vec<&Server> = nodes
        .iter()
        .filter(|node| supported_by(node).contains(&CoreKind::Xray))
        .collect();
    let Some(primary) = supported.first().copied() else {
        return Value::Object(Map::new());
    };
    let failover =
        settings.mode == ConnectionMode::Proxy && settings.failover.enabled && supported.len() > 1;
    let provider = ctx.provider.filter(|p| p.enabled);
    if let Some(provider) = provider {
        if provider.geoip_url.is_some() || provider.geosite_url.is_some() {
            tracing::warn!(
                "provider geo file URLs are not supported by Xray (cannot download); ignored"
            );
        }
    }
    let listen = if settings.allow_lan {
        "0.0.0.0"
    } else {
        "127.0.0.1"
    };

    let mut outbounds = Vec::new();
    if failover {
        for (i, node) in supported.iter().copied().enumerate() {
            outbounds.push(outbound_with_tag(node, settings, &format!("node-{i}")));
        }
    } else {
        outbounds.push(outbound(primary, settings));
    }
    outbounds.push(json!({"protocol": "freedom", "tag": "direct"}));
    outbounds.push(json!({"protocol": "blackhole", "tag": "block"}));

    let mut config = json!({
        "log": {"loglevel": settings.log_level},
        "api": {"tag": "api", "services": ["StatsService"]},
        "stats": {},
        "policy": {"system": {
            "statsInboundUplink": true,
            "statsInboundDownlink": true,
            "statsOutboundUplink": true,
            "statsOutboundDownlink": true,
        }},
        "dns": dns_config(settings, provider),
        "inbounds": inbounds(settings, listen, api_port, ctx.overrides),
        "outbounds": outbounds,
        "routing": routing_config(settings, provider, failover),
    });
    if failover {
        let selector: Vec<String> = (0..supported.len()).map(|i| format!("node-{i}")).collect();
        let strategy = match settings.failover.mode {
            FailoverMode::Failover => "leastPing",
            FailoverMode::LoadBalance => "roundRobin",
        };
        config["observatory"] = json!({
            "subjectSelector": selector.clone(),
            "probeUrl": crate::config::failover_probe_url(settings),
            "probeInterval": crate::config::duration_ms(crate::config::probe_interval_ms(settings)),
            "enableConcurrency": true,
        });
        config["routing"]["balancers"] = json!([{
            "tag": "proxy",
            "selector": selector,
            "strategy": {"type": strategy},
        }]);
    }
    config
}

/// Local inbounds (socks + http on `settings` ports, plus the stats `api` inbound).
fn inbounds(
    settings: &Settings,
    listen: &str,
    api_port: u16,
    overrides: Option<&BTreeMap<String, String>>,
) -> Value {
    let mut socks_settings = Map::new();
    socks_settings.insert("udp".into(), json!(settings.udp_enabled));
    match effective_local_auth("socks", settings, overrides) {
        Some(auth) => {
            socks_settings.insert("auth".into(), json!("password"));
            socks_settings.insert(
                "accounts".into(),
                json!([{"user": auth.user, "pass": auth.password}]),
            );
        }
        None => {
            socks_settings.insert("auth".into(), json!("noauth"));
        }
    }

    let mut http_settings = Map::new();
    if let Some(auth) = effective_local_auth("http", settings, overrides) {
        http_settings.insert(
            "accounts".into(),
            json!([{"user": auth.user, "pass": auth.password}]),
        );
    }

    // `sniffing` off (or a blank/absent override list) keeps `enabled: false`, which is Xray's
    // own default; when on, the sniffed protocols and route-only mode come from the settings.
    let sniffing = if settings.sniffing {
        json!({
            "enabled": true,
            "destOverride": settings.dest_override,
            "routeOnly": settings.route_only,
        })
    } else {
        json!({"enabled": false})
    };

    json!([
        {
            "tag": "socks",
            "listen": listen,
            "port": settings.socks_port,
            "protocol": "socks",
            "settings": Value::Object(socks_settings),
            "sniffing": sniffing,
        },
        {
            "tag": "http",
            "listen": listen,
            "port": settings.http_port,
            "protocol": "http",
            "settings": Value::Object(http_settings),
        },
        {
            "tag": "api",
            "listen": "127.0.0.1",
            "port": api_port,
            "protocol": "dokodemo-door",
            "settings": {"address": "127.0.0.1"},
        },
    ])
}

fn server_settings(server: &Server, username: &Option<String>, password: &Option<String>) -> Value {
    let mut srv = Map::new();
    srv.insert("address".into(), json!(server.address));
    srv.insert("port".into(), json!(server.port));
    if username.is_some() || password.is_some() {
        let user = json!({
            "user": username.clone().unwrap_or_default(),
            "pass": password.clone().unwrap_or_default(),
        });
        srv.insert("users".into(), json!([user]));
    }
    json!({"servers": [Value::Object(srv)]})
}

fn dns_config(settings: &Settings, provider: Option<&ProviderRouting>) -> Value {
    // A provider routing profile overrides remote/direct DNS and static hosts.
    let dns = provider
        .and_then(|p| p.dns.as_ref())
        .unwrap_or(&settings.dns);
    let direct =
        if provider.is_none() && settings.routing.preset == crate::model::RoutingPreset::BypassRu {
            json!({"address": dns.direct, "domains": ["geosite:category-ru"]})
        } else {
            json!(dns.direct)
        };
    let mut config = Map::new();

    // Static hosts: the user `dns.hosts` first, then the provider `dns_hosts` override per key
    // (a provider profile wins over the user setting, matching the remote/direct precedence).
    let mut hosts = settings.dns.hosts.clone();
    if let Some(provider) = provider {
        for (host, address) in &provider.dns_hosts {
            hosts.insert(host.clone(), address.clone());
        }
    }
    if !hosts.is_empty() {
        config.insert("hosts".into(), json!(hosts));
    }

    config.insert("servers".into(), json!([dns.remote, direct]));
    // Xray accepts `enableParallelQuery` / `serveStale` at the DNS level.
    if dns.parallel {
        config.insert("enableParallelQuery".into(), json!(true));
    }
    if dns.serve_stale {
        config.insert("serveStale".into(), json!(true));
    }
    // SVCB/HTTPS blocking and a bootstrap resolver are sing-box TUN features; Xray has no
    // equivalent fields here, so they are ignored (with a debug note) rather than failing.
    if dns.block_svcb {
        tracing::debug!("dns.block_svcb is sing-box-only; ignored by Xray");
    }
    if !dns.bootstrap.is_empty() {
        tracing::debug!("dns.bootstrap is sing-box-only; ignored by Xray");
    }
    Value::Object(config)
}

/// Routing block. `balancer` = failover is on and the tag `proxy` names the balancer, so
/// every Proxy rule must target it via `balancerTag` (an `outboundTag: "proxy"` rule would
/// resolve to a non-existent outbound). Without a balancer the output is unchanged.
fn routing_config(
    settings: &Settings,
    provider: Option<&ProviderRouting>,
    balancer: bool,
) -> Value {
    let mut rules = vec![json!({
        "type": "field",
        "inboundTag": ["api"],
        "outboundTag": "api",
    })];
    // Provider rules replace the user's preset and go before the user's custom rules.
    if let Some(provider) = provider {
        for rule in routing::provider_rules(provider) {
            rules.push(rule_json(&rule, balancer));
        }
    }
    for rule in routing::custom_rules(&settings.routing) {
        rules.push(rule_json(&rule, balancer));
    }
    if settings.routing.block_quic {
        rules.push(json!({
            "type": "field",
            "network": "udp",
            "port": "443",
            "outboundTag": "block",
        }));
    }
    // `global_proxy == false`: unmatched traffic goes direct via a catch-all rule
    // (Xray drops rules with no matcher fields, so match every transport).
    // The RF preset (no provider profile) falls back to DIRECT the same way.
    let direct_final = match provider {
        Some(p) => !p.global_proxy,
        None => routing::final_direct(&settings.routing),
    };
    match provider {
        Some(_) if direct_final => {
            rules.push(json!({
                "type": "field",
                "network": "tcp,udp",
                "outboundTag": "direct",
            }));
        }
        Some(_) => {}
        None => {
            for rule in routing::preset_rules(&settings.routing) {
                rules.push(rule_json(&rule, balancer));
            }
            if direct_final {
                rules.push(json!({
                    "type": "field",
                    "network": "tcp,udp",
                    "outboundTag": "direct",
                }));
            }
        }
    }
    // With a balancer, unmatched traffic would otherwise fall to the first outbound
    // (`node-0`); send it through the balancer too. The `global_proxy == false` direct
    // catch-all above already handles it, so it is not added there.
    if balancer && !direct_final {
        rules.push(json!({
            "type": "field",
            "network": "tcp,udp",
            "balancerTag": "proxy",
        }));
    }
    let domain_strategy = provider
        .and_then(|p| p.domain_strategy.clone())
        .unwrap_or_else(|| settings.routing.domain_strategy.clone());
    json!({
        "domainStrategy": domain_strategy,
        "rules": rules,
    })
}

/// One field rule. When `balancer` is set, a Proxy rule targets the balancer tag `proxy`
/// instead of an outbound with that name.
fn rule_json(rule: &Rule, balancer: bool) -> Value {
    let mut m = Map::new();
    m.insert("type".into(), json!("field"));
    if !rule.domains.is_empty() {
        m.insert("domain".into(), json!(rule.domains));
    }
    if !rule.ips.is_empty() {
        m.insert("ip".into(), json!(rule.ips));
    }
    if let Some(network) = non_empty(&rule.network) {
        m.insert("network".into(), json!(network));
    }
    if let Some(ports) = non_empty(&rule.ports) {
        m.insert("port".into(), json!(ports));
    }
    if !rule.processes.is_empty() {
        m.insert("process".into(), json!(rule.processes));
    }
    match rule.action {
        RuleAction::Proxy if balancer => {
            m.insert("balancerTag".into(), json!("proxy"));
        }
        RuleAction::Proxy => {
            m.insert("outboundTag".into(), json!("proxy"));
        }
        RuleAction::Direct => {
            m.insert("outboundTag".into(), json!("direct"));
        }
        RuleAction::Block => {
            m.insert("outboundTag".into(), json!("block"));
        }
    }
    Value::Object(m)
}

/// `None`/empty -> no field.
fn non_empty(value: &Option<String>) -> Option<&str> {
    value.as_deref().filter(|s| !s.is_empty())
}

fn add_mux(obj: &mut Map<String, Value>, settings: &Settings, allow: bool) {
    if settings.mux && allow {
        obj.insert(
            "mux".into(),
            json!({"enabled": true, "concurrency": settings.mux_concurrency}),
        );
    }
}

fn add_stream(
    obj: &mut Map<String, Value>,
    transport: &Transport,
    security: &Security,
    settings: &Settings,
) {
    let mut ss = Map::new();
    ss.insert(
        "network".into(),
        json!(match transport.network {
            Network::Tcp => "raw",
            Network::Ws => "ws",
            Network::Grpc => "grpc",
            Network::HttpUpgrade => "httpupgrade",
            Network::Xhttp => "xhttp",
            // Xray 26 removed the `http` and `quic` transports; both migrate to XHTTP.
            Network::H2 | Network::Quic => "xhttp",
            Network::Kcp => "kcp",
        }),
    );

    match security {
        Security::None => {}
        Security::Tls {
            sni,
            alpn,
            fingerprint,
            allow_insecure,
        } => {
            let mut tls = Map::new();
            if let Some(sni) = non_empty(sni) {
                tls.insert("serverName".into(), json!(sni));
            }
            if !alpn.is_empty() {
                tls.insert("alpn".into(), json!(alpn));
            }
            if let Some(fp) = resolve_fingerprint(fingerprint, settings) {
                tls.insert("fingerprint".into(), json!(fp));
            }
            if *allow_insecure {
                tls.insert("allowInsecure".into(), json!(true));
            }
            ss.insert("security".into(), json!("tls"));
            ss.insert("tlsSettings".into(), Value::Object(tls));
        }
        Security::Reality {
            sni,
            public_key,
            short_id,
            spider_x,
            fingerprint,
        } => {
            let mut tls = Map::new();
            if let Some(sni) = non_empty(sni) {
                tls.insert("serverName".into(), json!(sni));
            }
            tls.insert("publicKey".into(), json!(public_key));
            if let Some(short_id) = non_empty(short_id) {
                tls.insert("shortId".into(), json!(short_id));
            }
            if let Some(spider_x) = non_empty(spider_x) {
                tls.insert("spiderX".into(), json!(spider_x));
            }
            if let Some(fp) = resolve_fingerprint(fingerprint, settings) {
                tls.insert("fingerprint".into(), json!(fp));
            }
            ss.insert("security".into(), json!("reality"));
            ss.insert("realitySettings".into(), Value::Object(tls));
        }
    }

    match transport.network {
        Network::Ws => {
            let mut w = Map::new();
            if let Some(path) = non_empty(&transport.path) {
                w.insert("path".into(), json!(path));
            }
            if let Some(host) = non_empty(&transport.host) {
                w.insert("host".into(), json!(host));
            }
            ss.insert("wsSettings".into(), Value::Object(w));
        }
        Network::Grpc => {
            let mut g = Map::new();
            if let Some(service) = non_empty(&transport.service_name) {
                g.insert("serviceName".into(), json!(service));
            }
            if transport.mode.as_deref() == Some("multi") {
                g.insert("multiMode".into(), json!(true));
            }
            ss.insert("grpcSettings".into(), Value::Object(g));
        }
        Network::HttpUpgrade => {
            let mut h = Map::new();
            if let Some(path) = non_empty(&transport.path) {
                h.insert("path".into(), json!(path));
            }
            if let Some(host) = non_empty(&transport.host) {
                h.insert("host".into(), json!(host));
            }
            ss.insert("httpupgradeSettings".into(), Value::Object(h));
        }
        Network::Xhttp => {
            let mut x = Map::new();
            if let Some(path) = non_empty(&transport.path) {
                x.insert("path".into(), json!(path));
            }
            if let Some(host) = non_empty(&transport.host) {
                x.insert("host".into(), json!(host));
            }
            if let Some(mode) = non_empty(&transport.mode) {
                x.insert("mode".into(), json!(mode));
            }
            if let Some(extra) = &transport.extra {
                x.insert("extra".into(), extra.clone());
            }
            ss.insert("xhttpSettings".into(), Value::Object(x));
        }
        // Xray 26 removed the `http` transport; migrate an h2 link to XHTTP stream-one.
        Network::H2 => {
            let mut x = Map::new();
            if let Some(path) = non_empty(&transport.path) {
                x.insert("path".into(), json!(path));
            }
            if let Some(host) = non_empty(&transport.host) {
                x.insert("host".into(), json!(host));
            }
            x.insert("mode".into(), json!("stream-one"));
            ss.insert("xhttpSettings".into(), Value::Object(x));
        }
        Network::Tcp if transport.header_type.as_deref() == Some("http") => {
            let mut request = Map::new();
            if let Some(path) = non_empty(&transport.path) {
                request.insert("path".into(), json!([path]));
            }
            if let Some(host) = non_empty(&transport.host) {
                request.insert("headers".into(), json!({"Host": [host]}));
            }
            let mut header = Map::new();
            header.insert("type".into(), json!("http"));
            if !request.is_empty() {
                header.insert("request".into(), Value::Object(request));
            }
            ss.insert(
                "rawSettings".into(),
                json!({"header": Value::Object(header)}),
            );
        }
        // Xray 26 removed mKCP `header`/`seed` (migrated to finalmask); plain mKCP stays.
        Network::Kcp => {}
        // Xray 26 removed the `quic` transport; migrate to XHTTP stream-one.
        Network::Quic => {
            let mut x = Map::new();
            x.insert("mode".into(), json!("stream-one"));
            ss.insert("xhttpSettings".into(), Value::Object(x));
        }
        Network::Tcp => {}
    }

    obj.insert("streamSettings".into(), Value::Object(ss));
}

fn resolve_fingerprint(link: &Option<String>, settings: &Settings) -> Option<String> {
    link.clone().filter(|f| !f.is_empty()).or_else(|| {
        let default = settings.default_fingerprint.trim();
        if default.is_empty() {
            None
        } else {
            Some(default.to_string())
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Routing, RoutingPreset};

    fn server(credentials: Credentials) -> Server {
        Server {
            id: "test".into(),
            name: "test".into(),
            country: None,
            address: "example.com".into(),
            port: 443,
            credentials,
            transport: Transport::default(),
            security: Security::default(),
            raw_link: None,
            traffic: None,
            local_traffic: Default::default(),
            last_ping_ms: None,
        }
    }

    fn vless_server() -> Server {
        server(Credentials::Vless {
            uuid: "11111111-1111-1111-1111-111111111111".into(),
            flow: Some("xtls-rprx-vision".into()),
            encryption: Some("none".into()),
        })
    }

    #[test]
    fn vless_reality_vision() {
        let mut s = vless_server();
        s.security = Security::Reality {
            sni: Some("www.microsoft.com".into()),
            public_key: "PUBKEY".into(),
            short_id: Some("abcd".into()),
            spider_x: Some("/".into()),
            fingerprint: Some("chrome".into()),
        };
        let out = outbound(&s, &Settings::default());
        assert_eq!(out["tag"], json!("proxy"));
        assert_eq!(out["protocol"], json!("vless"));
        assert_eq!(
            out["settings"]["vnext"][0]["users"][0]["id"],
            json!("11111111-1111-1111-1111-111111111111")
        );
        assert_eq!(
            out["settings"]["vnext"][0]["users"][0]["flow"],
            json!("xtls-rprx-vision")
        );
        assert_eq!(
            out["settings"]["vnext"][0]["users"][0]["encryption"],
            json!("none")
        );
        assert_eq!(out["streamSettings"]["network"], json!("raw"));
        assert_eq!(out["streamSettings"]["security"], json!("reality"));
        assert_eq!(
            out["streamSettings"]["realitySettings"]["publicKey"],
            json!("PUBKEY")
        );
        assert_eq!(
            out["streamSettings"]["realitySettings"]["shortId"],
            json!("abcd")
        );
        assert_eq!(
            out["streamSettings"]["realitySettings"]["fingerprint"],
            json!("chrome")
        );
        assert!(out.get("mux").is_none(), "xtls flow must not enable mux");
    }

    #[test]
    fn vless_xhttp_with_extra() {
        let mut s = vless_server();
        s.credentials = Credentials::Vless {
            uuid: "11111111-1111-1111-1111-111111111111".into(),
            flow: None,
            encryption: None,
        };
        s.transport = Transport {
            network: Network::Xhttp,
            path: Some("/x".into()),
            host: Some("h.example".into()),
            mode: Some("packet-up".into()),
            extra: Some(json!({"noSSEHeader": true})),
            ..Transport::default()
        };
        s.security = Security::Tls {
            sni: Some("h.example".into()),
            alpn: vec!["h2".into()],
            fingerprint: None,
            allow_insecure: false,
        };
        let settings = Settings {
            mux: true,
            default_fingerprint: "firefox".into(),
            ..Settings::default()
        };

        let out = outbound(&s, &settings);
        assert_eq!(out["streamSettings"]["network"], json!("xhttp"));
        assert_eq!(out["streamSettings"]["xhttpSettings"]["path"], json!("/x"));
        assert_eq!(
            out["streamSettings"]["xhttpSettings"]["mode"],
            json!("packet-up")
        );
        assert_eq!(
            out["streamSettings"]["xhttpSettings"]["extra"]["noSSEHeader"],
            json!(true)
        );
        assert_eq!(out["streamSettings"]["security"], json!("tls"));
        assert_eq!(
            out["streamSettings"]["tlsSettings"]["fingerprint"],
            json!("firefox")
        );
        assert_eq!(out["streamSettings"]["tlsSettings"]["alpn"], json!(["h2"]));
        assert_eq!(out["mux"]["enabled"], json!(true));
    }

    #[test]
    fn vmess_ws_tls() {
        let mut s = server(Credentials::Vmess {
            uuid: "22222222-2222-2222-2222-222222222222".into(),
            alter_id: 0,
            cipher: Some("auto".into()),
        });
        s.port = 8443;
        s.transport = Transport {
            network: Network::Ws,
            path: Some("/ws".into()),
            host: Some("ws.example".into()),
            ..Transport::default()
        };
        s.security = Security::Tls {
            sni: Some("ws.example".into()),
            alpn: vec![],
            fingerprint: Some("chrome".into()),
            allow_insecure: true,
        };
        let out = outbound(&s, &Settings::default());
        assert_eq!(out["protocol"], json!("vmess"));
        assert_eq!(out["settings"]["vnext"][0]["port"], json!(8443));
        assert_eq!(out["settings"]["vnext"][0]["users"][0]["alterId"], json!(0));
        assert_eq!(
            out["settings"]["vnext"][0]["users"][0]["security"],
            json!("auto")
        );
        assert_eq!(out["streamSettings"]["network"], json!("ws"));
        assert_eq!(out["streamSettings"]["wsSettings"]["path"], json!("/ws"));
        assert_eq!(
            out["streamSettings"]["wsSettings"]["host"],
            json!("ws.example")
        );
        assert_eq!(out["streamSettings"]["security"], json!("tls"));
        assert_eq!(
            out["streamSettings"]["tlsSettings"]["allowInsecure"],
            json!(true)
        );
    }

    #[test]
    fn trojan_grpc() {
        let mut s = server(Credentials::Trojan {
            password: "secret".into(),
        });
        s.transport = Transport {
            network: Network::Grpc,
            service_name: Some("gsvc".into()),
            mode: Some("multi".into()),
            ..Transport::default()
        };
        let out = outbound(&s, &Settings::default());
        assert_eq!(out["protocol"], json!("trojan"));
        assert_eq!(out["settings"]["servers"][0]["password"], json!("secret"));
        assert_eq!(
            out["streamSettings"]["grpcSettings"]["serviceName"],
            json!("gsvc")
        );
        assert_eq!(
            out["streamSettings"]["grpcSettings"]["multiMode"],
            json!(true)
        );
    }

    #[test]
    fn shadowsocks_ignores_plugin() {
        let s = server(Credentials::Shadowsocks {
            method: "aes-256-gcm".into(),
            password: "pw".into(),
            plugin: Some("obfs-local".into()),
            plugin_opts: Some("obfs=http".into()),
        });
        let out = outbound(&s, &Settings::default());
        assert_eq!(out["protocol"], json!("shadowsocks"));
        assert_eq!(
            out["settings"]["servers"][0]["method"],
            json!("aes-256-gcm")
        );
        assert_eq!(out["settings"]["servers"][0]["password"], json!("pw"));
        assert!(out.get("streamSettings").is_none());
    }

    #[test]
    fn wireguard_outbound() {
        let s = server(Credentials::Wireguard {
            private_key: "priv".into(),
            peer_public_key: "pub".into(),
            pre_shared_key: Some("psk".into()),
            local_address: vec!["10.0.0.2/32".into()],
            reserved: Some([1, 2, 3]),
            mtu: Some(1420),
        });
        let out = outbound(&s, &Settings::default());
        assert_eq!(out["protocol"], json!("wireguard"));
        assert_eq!(out["settings"]["secretKey"], json!("priv"));
        assert_eq!(out["settings"]["address"], json!(["10.0.0.2/32"]));
        assert_eq!(out["settings"]["peers"][0]["publicKey"], json!("pub"));
        assert_eq!(out["settings"]["peers"][0]["preSharedKey"], json!("psk"));
        assert_eq!(
            out["settings"]["peers"][0]["endpoint"],
            json!("example.com:443")
        );
        assert_eq!(
            out["settings"]["peers"][0]["allowedIPs"],
            json!(["0.0.0.0/0", "::/0"])
        );
        assert_eq!(out["settings"]["reserved"], json!([1, 2, 3]));
        assert_eq!(out["settings"]["mtu"], json!(1420));
    }

    #[test]
    fn full_config_structure() {
        let settings = Settings {
            allow_lan: true,
            ..Settings::default()
        };
        let cfg = full_config(&vless_server(), &settings, 10818);
        assert_eq!(cfg["log"]["loglevel"], json!("warning"));
        assert_eq!(cfg["api"]["tag"], json!("api"));
        assert_eq!(cfg["api"]["services"], json!(["StatsService"]));
        assert_eq!(cfg["stats"], json!({}));
        assert_eq!(cfg["policy"]["system"]["statsInboundUplink"], json!(true));
        assert_eq!(
            cfg["policy"]["system"]["statsOutboundDownlink"],
            json!(true)
        );

        let inbounds = &cfg["inbounds"];
        assert_eq!(inbounds[0]["tag"], json!("socks"));
        assert_eq!(inbounds[0]["listen"], json!("0.0.0.0"));
        assert_eq!(inbounds[0]["port"], json!(10808));
        assert_eq!(inbounds[0]["settings"]["udp"], json!(true));
        assert_eq!(
            inbounds[0]["sniffing"]["destOverride"],
            json!(["http", "tls", "quic"])
        );
        assert_eq!(inbounds[0]["sniffing"]["routeOnly"], json!(true));
        // `parallel` defaults off, so no Xray parallel-query flag (matches the old behaviour).
        assert!(cfg["dns"].get("enableParallelQuery").is_none());
        assert!(cfg["dns"].get("serveStale").is_none());
        assert_eq!(inbounds[1]["tag"], json!("http"));
        assert_eq!(inbounds[1]["port"], json!(10809));
        assert_eq!(inbounds[2]["tag"], json!("api"));
        assert_eq!(inbounds[2]["port"], json!(10818));

        assert_eq!(cfg["outbounds"][0]["tag"], json!("proxy"));
        assert_eq!(cfg["outbounds"][1]["tag"], json!("direct"));
        assert_eq!(cfg["outbounds"][1]["protocol"], json!("freedom"));
        assert_eq!(cfg["outbounds"][2]["tag"], json!("block"));
        assert_eq!(cfg["outbounds"][2]["protocol"], json!("blackhole"));

        assert_eq!(cfg["routing"]["domainStrategy"], json!("IPIfNonMatch"));
        assert_eq!(cfg["routing"]["rules"][0]["inboundTag"], json!(["api"]));
        assert_eq!(cfg["routing"]["rules"][0]["outboundTag"], json!("api"));
    }

    #[test]
    fn full_config_bypass_ru_dns_and_rules() {
        let settings = Settings {
            routing: Routing {
                preset: RoutingPreset::BypassRu,
                block_quic: true,
                ..Routing::default()
            },
            ..Settings::default()
        };
        let cfg = full_config(&vless_server(), &settings, 10818);
        assert_eq!(cfg["dns"]["servers"][0], json!("https://1.1.1.1/dns-query"));
        assert_eq!(
            cfg["dns"]["servers"][1]["domains"],
            json!(["geosite:category-ru"])
        );
        assert_eq!(
            cfg["dns"]["servers"][1]["address"],
            json!("https://77.88.8.8/dns-query")
        );

        let rules = cfg["routing"]["rules"].as_array().unwrap();
        // api rule, block-quic rule, then the RF rules (data order), then a direct catch-all.
        assert!(rules.iter().any(|r| r["network"] == json!("udp")
            && r["port"] == json!("443")
            && r["outboundTag"] == json!("block")));
        assert_eq!(rules[2]["port"], json!("53"));
        assert!(rules
            .iter()
            .any(|r| r["outboundTag"] == json!("direct") && r["ip"] == json!(["geoip:ru"])));
        assert!(rules.iter().any(|r| r["outboundTag"] == json!("proxy")
            && r["domain"]
                .as_array()
                .is_some_and(|d| d.contains(&json!("geosite:ru-blocked")))));
        assert!(rules.iter().any(|r| r["process"].is_array()));
        // Remote text rule lists are skipped: no rule mentions them.
        assert!(!rules.iter().any(|r| r.to_string().contains("itdog")));
        let last = rules.last().unwrap();
        assert_eq!(last["outboundTag"], json!("direct"));
        assert_eq!(last["network"], json!("tcp,udp"));
    }

    #[test]
    fn full_config_global_private_direct() {
        let settings = Settings {
            routing: Routing {
                preset: RoutingPreset::Global,
                ..Routing::default()
            },
            ..Settings::default()
        };
        let cfg = full_config(&vless_server(), &settings, 10818);
        let rules = cfg["routing"]["rules"].as_array().unwrap();
        assert!(rules
            .iter()
            .any(|r| r["ip"] == json!(["geoip:private"]) && r["outboundTag"] == json!("direct")));
        assert_eq!(cfg["outbounds"][0]["tag"], json!("proxy"));
    }

    use crate::model::{Dns, ProviderRouting, RoutingRule};

    fn provider_profile(global_proxy: bool, rules: Vec<RoutingRule>) -> ProviderRouting {
        ProviderRouting {
            name: "provider".into(),
            enabled: true,
            global_proxy,
            rules,
            ..ProviderRouting::default()
        }
    }

    fn rule(domains: Vec<&str>, action: RuleAction) -> RoutingRule {
        RoutingRule {
            domains: domains.into_iter().map(str::to_string).collect(),
            ips: Vec::new(),
            processes: Vec::new(),
            network: None,
            ports: None,
            action,
            enabled: true,
        }
    }

    fn ctx_of(provider: &ProviderRouting) -> ConnectContext<'_> {
        ConnectContext {
            provider: Some(provider),
            overrides: None,
        }
    }

    #[test]
    fn provider_rules_replace_preset_and_come_before_custom() {
        let settings = Settings {
            routing: Routing {
                preset: RoutingPreset::BypassRu,
                custom_rules: vec![rule(vec!["domain:custom.example"], RuleAction::Direct)],
                ..Routing::default()
            },
            ..Settings::default()
        };
        let provider =
            provider_profile(true, vec![rule(vec!["geosite:google"], RuleAction::Proxy)]);
        let cfg = full_config_with(&vless_server(), &settings, 10818, &ctx_of(&provider));
        let rules = cfg["routing"]["rules"].as_array().unwrap();
        let tags: Vec<&str> = rules
            .iter()
            .filter_map(|r| r["outboundTag"].as_str())
            .collect();
        // api, provider(proxy), custom(direct); no block-quic, no preset.
        assert_eq!(tags, ["api", "proxy", "direct"]);
        assert!(rules
            .iter()
            .any(|r| r["domain"] == json!(["geosite:google"])));
        assert!(rules
            .iter()
            .any(|r| r["domain"] == json!(["domain:custom.example"])));
        // BypassRu preset must be gone.
        assert!(!rules
            .iter()
            .any(|r| r["domain"] == json!(["geosite:category-ads-all"])));
    }

    #[test]
    fn provider_global_proxy_false_adds_final_direct() {
        let provider = provider_profile(false, Vec::new());
        let cfg = full_config_with(
            &vless_server(),
            &Settings::default(),
            10818,
            &ctx_of(&provider),
        );
        let rules = cfg["routing"]["rules"].as_array().unwrap();
        let last = rules.last().unwrap();
        assert_eq!(last["outboundTag"], json!("direct"));
        assert_eq!(last["network"], json!("tcp,udp"));
        assert!(last.get("domain").is_none() && last.get("ip").is_none());

        // global_proxy = true: unmatched traffic still reaches the proxy (no catch-all).
        let provider = provider_profile(true, Vec::new());
        let cfg = full_config_with(
            &vless_server(),
            &Settings::default(),
            10818,
            &ctx_of(&provider),
        );
        let rules = cfg["routing"]["rules"].as_array().unwrap();
        assert_eq!(rules.len(), 1); // just the api rule
    }

    #[test]
    fn provider_dns_hosts_and_domain_strategy_override() {
        let mut provider = provider_profile(true, Vec::new());
        provider.dns = Some(Dns {
            remote: "https://9.9.9.9/dns-query".into(),
            direct: "https://8.8.8.8/dns-query".into(),
            fake_ip: false,
            ..Dns::default()
        });
        provider
            .dns_hosts
            .insert("example.com".into(), "10.0.0.1".into());
        provider.domain_strategy = Some("AsIs".into());
        let settings = Settings {
            routing: Routing {
                domain_strategy: "IPIfNonMatch".into(),
                ..Routing::default()
            },
            ..Settings::default()
        };
        let cfg = full_config_with(&vless_server(), &settings, 10818, &ctx_of(&provider));
        assert_eq!(cfg["dns"]["servers"][0], json!("https://9.9.9.9/dns-query"));
        assert_eq!(cfg["dns"]["servers"][1], json!("https://8.8.8.8/dns-query"));
        assert_eq!(cfg["dns"]["hosts"]["example.com"], json!("10.0.0.1"));
        assert_eq!(cfg["routing"]["domainStrategy"], json!("AsIs"));
    }

    #[test]
    fn override_enables_local_auth() {
        let overrides = BTreeMap::from([
            ("socks-auth-mode".to_string(), "manual".to_string()),
            ("socks-auth-user".to_string(), "u".to_string()),
            ("socks-auth-password".to_string(), "p".to_string()),
            ("http-auth-mode".to_string(), "manual".to_string()),
            ("http-auth-user".to_string(), "hu".to_string()),
            ("http-auth-password".to_string(), "hp".to_string()),
        ]);
        let ctx = ConnectContext {
            provider: None,
            overrides: Some(&overrides),
        };
        let cfg = full_config_with(&vless_server(), &Settings::default(), 10818, &ctx);
        let socks = &cfg["inbounds"][0];
        assert_eq!(socks["settings"]["auth"], json!("password"));
        assert_eq!(socks["settings"]["accounts"][0]["user"], json!("u"));
        assert_eq!(socks["settings"]["accounts"][0]["pass"], json!("p"));
        let http = &cfg["inbounds"][1];
        assert_eq!(http["settings"]["accounts"][0]["user"], json!("hu"));
        assert_eq!(http["settings"]["accounts"][0]["pass"], json!("hp"));

        // mode != manual (or absent/blank creds) leaves the socks inbound unauthenticated.
        let plain = BTreeMap::from([("socks-auth-mode".to_string(), "disable".to_string())]);
        let ctx = ConnectContext {
            provider: None,
            overrides: Some(&plain),
        };
        let cfg = full_config_with(&vless_server(), &Settings::default(), 10818, &ctx);
        assert_eq!(cfg["inbounds"][0]["settings"]["auth"], json!("noauth"));
    }

    #[test]
    fn inbound_udp_sniffing_and_user_auth() {
        let settings = Settings {
            udp_enabled: false,
            sniffing: false,
            local_auth_user: "u".into(),
            local_auth_pass: "p".into(),
            ..Settings::default()
        };
        let cfg = full_config(&vless_server(), &settings, 10818);
        assert_eq!(cfg["inbounds"][0]["settings"]["udp"], json!(false));
        assert_eq!(cfg["inbounds"][0]["sniffing"]["enabled"], json!(false));
        assert_eq!(cfg["inbounds"][0]["settings"]["auth"], json!("password"));
        assert_eq!(
            cfg["inbounds"][0]["settings"]["accounts"][0]["user"],
            json!("u")
        );
        assert_eq!(
            cfg["inbounds"][1]["settings"]["accounts"][0]["pass"],
            json!("p")
        );
    }

    #[test]
    fn dest_override_and_route_only_wire_through() {
        let settings = Settings {
            dest_override: vec!["http".into(), "tls".into(), "quic".into(), "fakedns".into()],
            route_only: true,
            ..Settings::default()
        };
        let cfg = full_config(&vless_server(), &settings, 10818);
        assert_eq!(
            cfg["inbounds"][0]["sniffing"]["destOverride"],
            json!(["http", "tls", "quic", "fakedns"])
        );
        assert_eq!(cfg["inbounds"][0]["sniffing"]["routeOnly"], json!(true));
    }

    #[test]
    fn mux_concurrency_wired_only_when_enabled() {
        let s = server(Credentials::Vless {
            uuid: "u".into(),
            flow: None,
            encryption: None,
        });
        let off = outbound(&s, &Settings::default());
        assert!(off.get("mux").is_none());
        let on = outbound(
            &s,
            &Settings {
                mux: true,
                mux_concurrency: 16,
                ..Settings::default()
            },
        );
        assert_eq!(on["mux"]["enabled"], json!(true));
        assert_eq!(on["mux"]["concurrency"], json!(16));
    }

    #[test]
    fn dns_hosts_parallel_and_serve_stale() {
        let mut settings = Settings {
            dns: Dns {
                parallel: true,
                serve_stale: true,
                ..Dns::default()
            },
            ..Settings::default()
        };
        settings
            .dns
            .hosts
            .insert("example.com".into(), "10.0.0.1".into());
        let cfg = full_config(&vless_server(), &settings, 10818);
        assert_eq!(cfg["dns"]["hosts"]["example.com"], json!("10.0.0.1"));
        assert_eq!(cfg["dns"]["enableParallelQuery"], json!(true));
        assert_eq!(cfg["dns"]["serveStale"], json!(true));

        // A provider `dns_hosts` entry overrides the user host for the same key.
        let mut provider = provider_profile(true, Vec::new());
        provider
            .dns_hosts
            .insert("example.com".into(), "10.0.0.9".into());
        let cfg = full_config_with(&vless_server(), &settings, 10818, &ctx_of(&provider));
        assert_eq!(cfg["dns"]["hosts"]["example.com"], json!("10.0.0.9"));
    }

    use crate::model::Failover;

    fn failover_settings() -> Settings {
        Settings {
            failover: Failover {
                enabled: true,
                ..Failover::default()
            },
            ..Settings::default()
        }
    }

    fn proxy_rule(domain: &str) -> RoutingRule {
        RoutingRule {
            domains: vec![format!("domain:{domain}")],
            ips: Vec::new(),
            processes: Vec::new(),
            network: None,
            ports: None,
            action: RuleAction::Proxy,
            enabled: true,
        }
    }

    #[test]
    fn failover_custom_proxy_rule_uses_balancer_tag() {
        let nodes = [vless_server(), vless_server()];
        let mut settings = failover_settings();
        settings.routing.preset = RoutingPreset::Custom;
        settings.routing.custom_rules = vec![proxy_rule("proxy.example")];
        let cfg = full_config_cluster_with(&nodes, &settings, 10818, &ConnectContext::default());
        let rules = cfg["routing"]["rules"].as_array().unwrap();
        // api, custom (balancerTag), final balancer catch-all.
        assert_eq!(rules[0]["outboundTag"], json!("api"));
        assert_eq!(rules[1]["balancerTag"], json!("proxy"));
        assert!(rules[1].get("outboundTag").is_none());
        let last = rules.last().unwrap();
        assert_eq!(last["balancerTag"], json!("proxy"));
        assert_eq!(last["network"], json!("tcp,udp"));
        // No rule may target a non-existent `proxy` outbound.
        assert!(!rules.iter().any(|r| r["outboundTag"] == json!("proxy")));
    }

    #[test]
    fn failover_preset_direct_and_block_rules_unchanged() {
        let nodes = [vless_server(), vless_server()];
        let mut settings = failover_settings();
        settings.routing.preset = RoutingPreset::Global;
        let cfg = full_config_cluster_with(&nodes, &settings, 10818, &ConnectContext::default());
        let rules = cfg["routing"]["rules"].as_array().unwrap();
        assert!(rules
            .iter()
            .any(|r| r["outboundTag"] == json!("direct") && r["ip"] == json!(["geoip:private"])));
        // Only the final catch-all uses the balancer.
        let last = rules.last().unwrap();
        assert_eq!(last["balancerTag"], json!("proxy"));
        assert_eq!(
            rules
                .iter()
                .filter(|r| r.get("balancerTag").is_some())
                .count(),
            1
        );
    }

    #[test]
    fn failover_rf_preset_proxy_rules_use_balancer_and_final_is_direct() {
        let nodes = [vless_server(), vless_server()];
        let settings = failover_settings(); // RF (BypassRu) preset
        let cfg = full_config_cluster_with(&nodes, &settings, 10818, &ConnectContext::default());
        let rules = cfg["routing"]["rules"].as_array().unwrap();
        assert!(rules.iter().any(|r| r["balancerTag"] == json!("proxy")));
        assert!(!rules.iter().any(|r| r["outboundTag"] == json!("proxy")));
        assert_eq!(rules.last().unwrap()["outboundTag"], json!("direct"));
    }

    #[test]
    fn failover_provider_global_direct_keeps_direct_catch_all() {
        let nodes = [vless_server(), vless_server()];
        let settings = failover_settings();
        // Provider asks for global-direct and contributes a proxy rule.
        let provider = provider_profile(false, vec![proxy_rule("provider.example")]);
        let cfg = full_config_cluster_with(&nodes, &settings, 10818, &ctx_of(&provider));
        let rules = cfg["routing"]["rules"].as_array().unwrap();
        // Provider proxy rule -> balancerTag.
        assert!(rules.iter().any(|r| r["balancerTag"] == json!("proxy")
            && r["domain"] == json!(["domain:provider.example"])));
        // The direct catch-all stays LAST; no balancer catch-all after it.
        let last = rules.last().unwrap();
        assert_eq!(last["outboundTag"], json!("direct"));
        assert_eq!(last["network"], json!("tcp,udp"));
        assert!(last.get("balancerTag").is_none());
        assert!(!rules.iter().any(|r| r["outboundTag"] == json!("proxy")));
    }

    #[test]
    fn failover_provider_global_proxy_adds_balancer_catch_all() {
        let nodes = [vless_server(), vless_server()];
        let settings = failover_settings();
        let provider = provider_profile(true, Vec::new());
        let cfg = full_config_cluster_with(&nodes, &settings, 10818, &ctx_of(&provider));
        let rules = cfg["routing"]["rules"].as_array().unwrap();
        // api rule + final balancer catch-all.
        assert_eq!(rules.len(), 2);
        let last = rules.last().unwrap();
        assert_eq!(last["balancerTag"], json!("proxy"));
        assert_eq!(last["network"], json!("tcp,udp"));
    }

    #[test]
    fn failover_off_routing_is_unchanged() {
        let nodes = [vless_server(), vless_server()];
        let mut settings = Settings::default();
        settings.routing.preset = RoutingPreset::Custom;
        settings.routing.custom_rules = vec![proxy_rule("proxy.example")];
        // Failover disabled: the cluster config's routing equals the single-server one.
        let cluster =
            full_config_cluster_with(&nodes, &settings, 10818, &ConnectContext::default());
        let single = full_config_with(&nodes[0], &settings, 10818, &ConnectContext::default());
        assert_eq!(cluster["routing"], single["routing"]);
        let rules = cluster["routing"]["rules"].as_array().unwrap();
        assert!(rules.iter().any(|r| r["outboundTag"] == json!("proxy")));
        assert!(rules.iter().all(|r| r.get("balancerTag").is_none()));
    }
}

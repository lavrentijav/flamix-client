//! sing-box JSON config (1.12+ format). OWNER: task T03.
//!
//! Uses the current sing-box format: DNS servers as `{type, server, tag}` objects,
//! remote `.srs` rule sets instead of the removed `geoip`/`geosite` fields, and
//! rule actions (`reject`, `hijack-dns`, `sniff`) instead of the legacy `block` /
//! `dns` special outbounds. WireGuard is returned as an `endpoints` entry (sing-box
//! 1.12 moved WireGuard out of `outbounds`); see [`outbound`].

use crate::config::routing::{self, Rule};
use crate::config::{effective_local_auth, ConnectContext};
use crate::model::{
    Credentials, Network, ProviderRouting, RoutingPreset, RuleAction, Security, Server, Settings,
    Transport,
};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;

const PRIVATE_RANGES: [&str; 9] = [
    "10.0.0.0/8",
    "172.16.0.0/12",
    "192.168.0.0/16",
    "127.0.0.0/8",
    "169.254.0.0/16",
    "::1/128",
    "fc00::/7",
    "fe80::/10",
    "224.0.0.0/4",
];

/// Outbound object (`"tag": "proxy"`) for `server`; `None` if sing-box can't express it.
///
/// For WireGuard the returned object is an **endpoint** (sing-box 1.12): put it in
/// the top-level `endpoints` array, not in `outbounds`.
pub fn outbound(server: &Server, settings: &Settings) -> Option<Value> {
    if matches!(
        server.transport.network,
        Network::Xhttp | Network::Kcp | Network::Quic
    ) {
        return None;
    }

    let mut obj = Map::new();
    obj.insert("tag".into(), json!("proxy"));

    match &server.credentials {
        Credentials::Vless { uuid, flow, .. } => {
            obj.insert("type".into(), json!("vless"));
            obj.insert("server".into(), json!(server.address));
            obj.insert("server_port".into(), json!(server.port));
            obj.insert("uuid".into(), json!(uuid));
            if let Some(flow) = non_empty(flow) {
                obj.insert("flow".into(), json!(flow));
            }
        }
        Credentials::Vmess {
            uuid,
            alter_id,
            cipher,
        } => {
            obj.insert("type".into(), json!("vmess"));
            obj.insert("server".into(), json!(server.address));
            obj.insert("server_port".into(), json!(server.port));
            obj.insert("uuid".into(), json!(uuid));
            obj.insert("alter_id".into(), json!(alter_id));
            obj.insert(
                "security".into(),
                json!(cipher.clone().unwrap_or_else(|| "auto".into())),
            );
        }
        Credentials::Trojan { password } => {
            obj.insert("type".into(), json!("trojan"));
            obj.insert("server".into(), json!(server.address));
            obj.insert("server_port".into(), json!(server.port));
            obj.insert("password".into(), json!(password));
        }
        Credentials::Shadowsocks {
            method, password, ..
        } => {
            obj.insert("type".into(), json!("shadowsocks"));
            obj.insert("server".into(), json!(server.address));
            obj.insert("server_port".into(), json!(server.port));
            obj.insert("method".into(), json!(method));
            obj.insert("password".into(), json!(password));
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
            obj.insert("server".into(), json!(server.address));
            obj.insert("server_port".into(), json!(server.port));
            obj.insert("password".into(), json!(password));
            if let Some(obfs) = non_empty(obfs) {
                let mut value = Map::new();
                value.insert("type".into(), json!(obfs));
                if let Some(password) = non_empty(obfs_password) {
                    value.insert("password".into(), json!(password));
                }
                obj.insert("obfs".into(), Value::Object(value));
            }
            if let Some(ports) = non_empty(ports) {
                let list = parse_server_ports(ports);
                if !list.is_empty() {
                    obj.insert("server_ports".into(), json!(list));
                }
            }
            if let Some(up) = up_mbps {
                obj.insert("up_mbps".into(), json!(up));
            }
            if let Some(down) = down_mbps {
                obj.insert("down_mbps".into(), json!(down));
            }
        }
        Credentials::Tuic {
            uuid,
            password,
            congestion_control,
            udp_relay_mode,
        } => {
            obj.insert("type".into(), json!("tuic"));
            obj.insert("server".into(), json!(server.address));
            obj.insert("server_port".into(), json!(server.port));
            obj.insert("uuid".into(), json!(uuid));
            obj.insert("password".into(), json!(password));
            if let Some(cc) = non_empty(congestion_control) {
                obj.insert("congestion_control".into(), json!(cc));
            }
            if let Some(mode) = non_empty(udp_relay_mode) {
                obj.insert("udp_relay_mode".into(), json!(mode));
            }
        }
        Credentials::Wireguard {
            private_key,
            peer_public_key,
            pre_shared_key,
            local_address,
            reserved,
            mtu,
        } => {
            obj.insert("type".into(), json!("wireguard"));
            if let Some(mtu) = mtu {
                obj.insert("mtu".into(), json!(mtu));
            }
            if !local_address.is_empty() {
                obj.insert("address".into(), json!(local_address));
            }
            obj.insert("private_key".into(), json!(private_key));
            let mut peer = Map::new();
            peer.insert("address".into(), json!(server.address));
            peer.insert("port".into(), json!(server.port));
            peer.insert("public_key".into(), json!(peer_public_key));
            if let Some(psk) = non_empty(pre_shared_key) {
                peer.insert("pre_shared_key".into(), json!(psk));
            }
            peer.insert("allowed_ips".into(), json!(["0.0.0.0/0", "::/0"]));
            if let Some(reserved) = reserved {
                peer.insert("reserved".into(), json!(reserved));
            }
            obj.insert("peers".into(), json!([Value::Object(peer)]));
            return Some(Value::Object(obj));
        }
        Credentials::Socks { username, password } => {
            obj.insert("type".into(), json!("socks"));
            obj.insert("server".into(), json!(server.address));
            obj.insert("server_port".into(), json!(server.port));
            obj.insert("version".into(), json!("5"));
            if let Some(username) = non_empty(username) {
                obj.insert("username".into(), json!(username));
            }
            if let Some(password) = non_empty(password) {
                obj.insert("password".into(), json!(password));
            }
        }
        Credentials::Http { username, password } => {
            obj.insert("type".into(), json!("http"));
            obj.insert("server".into(), json!(server.address));
            obj.insert("server_port".into(), json!(server.port));
            if let Some(username) = non_empty(username) {
                obj.insert("username".into(), json!(username));
            }
            if let Some(password) = non_empty(password) {
                obj.insert("password".into(), json!(password));
            }
        }
    }

    // hysteria2 / tuic are TLS-only transports.
    let tls_required = matches!(
        server.credentials,
        Credentials::Hysteria2 { .. } | Credentials::Tuic { .. }
    );
    if let Some(tls) = tls_json(&server.security, settings, tls_required) {
        obj.insert("tls".into(), tls);
    }
    if let Some(transport) = transport_json(&server.transport) {
        obj.insert("transport".into(), transport);
    }
    // sing-box 1.14: dialing a domain needs an explicit resolver.
    if is_domain(&server.address) {
        obj.insert("domain_resolver".into(), json!("direct"));
    }
    // Outbound multiplexing, wired only for the protocols sing-box supports it on. Xray's
    // `concurrency` maps to sing-box `max_streams` (both = concurrent streams per connection).
    if settings.mux && mux_supported(server) {
        let mut mux = Map::new();
        mux.insert("enabled".into(), json!(true));
        if settings.mux_concurrency > 0 {
            mux.insert("max_streams".into(), json!(settings.mux_concurrency));
        }
        obj.insert("multiplex".into(), Value::Object(mux));
    }

    Some(Value::Object(obj))
}

/// True when `multiplex` may be emitted for `server` (vless without an xtls flow, vmess,
/// trojan, shadowsocks; hysteria2/tuic/wireguard/socks/http manage their own multiplexing).
fn mux_supported(server: &Server) -> bool {
    match &server.credentials {
        Credentials::Vless { flow, .. } => flow.as_deref().unwrap_or("").is_empty(),
        Credentials::Vmess { .. }
        | Credentials::Trojan { .. }
        | Credentials::Shadowsocks { .. } => true,
        _ => false,
    }
}

/// True when `value` is not a literal IP address (so it must be resolved).
fn is_domain(value: &str) -> bool {
    value.parse::<std::net::IpAddr>().is_err()
}

/// Map Xray `destOverride` values to sing-box `sniff` action `sniffer` names.
///
/// sing-box sniffs all protocols unless a `sniffer` list is given; `fakedns` has no sing-box
/// sniffer and is dropped (the fake-ip range is configured separately on the `fakeip` server).
fn singbox_sniffers(dest_override: &[String]) -> Vec<&'static str> {
    dest_override
        .iter()
        .filter_map(|value| match value.to_ascii_lowercase().as_str() {
            "http" => Some("http"),
            "tls" => Some("tls"),
            "quic" => Some("quic"),
            _ => None,
        })
        .collect()
}

/// A failover node outbound: the real outbound for `node`, tagged `node-<index>`.
fn node_outbound(node: &Server, settings: &Settings, index: usize) -> Value {
    let tag = format!("node-{index}");
    match outbound(node, settings) {
        Some(mut out) => {
            if let Value::Object(map) = &mut out {
                map.insert("tag".into(), json!(tag));
            }
            out
        }
        // Unreachable for clusterable nodes; a direct hop keeps the tag resolvable.
        None => json!({"type": "direct", "tag": tag}),
    }
}

/// The `urltest` balancer outbound, tagged `proxy` so all existing references keep working.
fn urltest_outbound(node_count: usize, settings: &Settings) -> Value {
    let outbounds: Vec<String> = (0..node_count).map(|i| format!("node-{i}")).collect();
    json!({
        "type": "urltest",
        "tag": "proxy",
        "outbounds": outbounds,
        "url": crate::config::failover_probe_url(settings),
        "interval": crate::config::duration_ms(crate::config::probe_interval_ms(settings)),
        "tolerance": settings.failover.tolerance_ms,
    })
}

/// True when `server` can be a member of a sing-box `urltest` cluster.
///
/// WireGuard is emitted as an endpoint (not an `outbounds` entry) and xhttp/kcp/quic have no
/// sing-box outbound, so neither can take part.
pub(crate) fn clusterable(server: &Server, settings: &Settings) -> bool {
    !matches!(server.credentials, Credentials::Wireguard { .. })
        && outbound(server, settings).is_some()
}

/// Full config. `tun` = add a tun inbound (auto_route, strict_route) instead of mixed.
/// `upstream_socks` = Some(port): the proxy outbound is SOCKS to 127.0.0.1:port (Xray chain).
/// `api_port`: `experimental.clash_api` on 127.0.0.1 for traffic stats.
pub fn full_config(
    server: &Server,
    settings: &Settings,
    tun: bool,
    upstream_socks: Option<u16>,
    api_port: u16,
) -> Value {
    full_config_with(
        server,
        settings,
        tun,
        upstream_socks,
        api_port,
        &ConnectContext::default(),
    )
}

/// Like [`full_config`], but with the selected subscription's provider routing and Happ
/// app-overrides ([`ConnectContext`]). See [`crate::config::plan_with`].
pub fn full_config_with(
    server: &Server,
    settings: &Settings,
    tun: bool,
    upstream_socks: Option<u16>,
    api_port: u16,
    ctx: &ConnectContext,
) -> Value {
    build_config(
        server,
        &[server],
        settings,
        tun,
        upstream_socks,
        api_port,
        ctx,
    )
}

/// Full config for a primary node plus ordered backups (`node-0` is the primary).
///
/// With one proxy-capable node this is exactly [`full_config_with`]. With
/// `settings.failover.enabled` and more than one node, every node becomes an outbound tagged
/// `node-N` and a `urltest` outbound tagged `proxy` is added; `route.final` and the DNS/
/// rule-set `detour` references keep pointing at `proxy`. Provider routing/overrides belong to
/// the primary node and apply to the whole config.
///
/// Defensive: nodes sing-box cannot express (WireGuard, xhttp/kcp/quic) are skipped — the same
/// drop-incompatible-keep-compatible policy as [`crate::config::plan_cluster`], which also
/// warns about the dropped nodes.
pub(crate) fn full_config_cluster_with(
    nodes: &[Server],
    settings: &Settings,
    tun: bool,
    upstream_socks: Option<u16>,
    api_port: u16,
    ctx: &ConnectContext,
) -> Value {
    let compatible: Vec<&Server> = nodes
        .iter()
        .filter(|node| clusterable(node, settings))
        .collect();
    let Some(primary) = compatible.first().copied() else {
        return Value::Object(Map::new());
    };
    let cluster =
        settings.failover.enabled && compatible.len() > 1 && !tun && upstream_socks.is_none();
    if !cluster {
        return full_config_with(primary, settings, tun, upstream_socks, api_port, ctx);
    }
    build_config(
        primary,
        &compatible,
        settings,
        tun,
        upstream_socks,
        api_port,
        ctx,
    )
}

/// Builds one sing-box config for `primary`; `nodes` is the failover path
/// (`nodes.len() > 1` adds the `urltest` balancer tagged `proxy`).
fn build_config(
    primary: &Server,
    nodes: &[&Server],
    settings: &Settings,
    tun: bool,
    upstream_socks: Option<u16>,
    api_port: u16,
    ctx: &ConnectContext,
) -> Value {
    let provider = ctx.provider.filter(|p| p.enabled);
    let provider_dns = provider.and_then(|p| p.dns.as_ref());
    let remote_dns = provider_dns
        .map(|d| d.remote.as_str())
        .unwrap_or(&settings.dns.remote);
    let direct_dns = provider_dns
        .map(|d| d.direct.as_str())
        .unwrap_or(&settings.dns.direct);
    let fake_ip = provider.map(|p| p.fake_dns).unwrap_or(settings.dns.fake_ip);
    let fake_ip_range = provider_dns
        .map(|d| d.fake_ip_range.as_str())
        .unwrap_or(&settings.dns.fake_ip_range);
    let bootstrap = provider_dns
        .map(|d| d.bootstrap.as_str())
        .unwrap_or(&settings.dns.bootstrap)
        .trim();
    let block_svcb = provider_dns
        .map(|d| d.block_svcb)
        .unwrap_or(settings.dns.block_svcb);
    // sing-box has no parallel-query switch and no Xray `serveStale`; both are ignored.
    if provider_dns
        .map(|d| d.parallel)
        .unwrap_or(settings.dns.parallel)
    {
        tracing::debug!("dns.parallel is Xray-only; ignored by sing-box");
    }
    if provider_dns
        .map(|d| d.serve_stale)
        .unwrap_or(settings.dns.serve_stale)
    {
        tracing::debug!("dns.serve_stale is Xray-only; ignored by sing-box");
    }
    let geo = GeoUrls::new(settings, provider);

    let wireguard = matches!(primary.credentials, Credentials::Wireguard { .. });
    let proxy_out = match upstream_socks {
        Some(port) => json!({
            "type": "socks",
            "tag": "proxy",
            "server": "127.0.0.1",
            "server_port": port,
            "version": "5",
        }),
        None => {
            outbound(primary, settings).unwrap_or_else(|| json!({"type": "direct", "tag": "proxy"}))
        }
    };
    let proxy_in_endpoints = wireguard && upstream_socks.is_none();

    let mut rule_sets: BTreeMap<String, String> = BTreeMap::new();
    let mut route_rules = Vec::new();
    if settings.sniffing {
        let mut sniff = Map::new();
        sniff.insert("action".into(), json!("sniff"));
        let sniffer: Vec<&str> = singbox_sniffers(&settings.dest_override);
        if !sniffer.is_empty() {
            sniff.insert("sniffer".into(), json!(sniffer));
        }
        route_rules.push(Value::Object(sniff));
        // LIMITATION: `route_only` (Xray: sniff for routing but do not override the destination
        // the outbound dials) has no sing-box equivalent. The `sniff` action always makes the
        // sniffed domain the destination, and sing-box has no "sniff without override" switch,
        // so the flag is ignored (noted for the user, never a config failure).
        if settings.route_only {
            tracing::debug!("settings.route_only is Xray-only; ignored by sing-box");
        }
    } else {
        tracing::debug!("settings.sniffing=false: omitting the sing-box `sniff` action");
    }
    route_rules.push(json!({"protocol": "dns", "action": "hijack-dns"}));
    if tun {
        // Prevent the chained Xray/sing-box processes from being routed into the tunnel.
        route_rules.push(json!({
            "process_name": ["xray", "xray.exe", "sing-box", "sing-box.exe"],
            "outbound": "direct",
        }));
        // ICMP routing; `rule` (the neutral value) leaves ICMP to normal routing.
        match settings.tun_icmp.as_str() {
            "direct" => route_rules.push(json!({"network": "icmp", "outbound": "direct"})),
            "unreachable" => route_rules.push(json!({"network": "icmp", "action": "reject"})),
            "drop" => {
                route_rules.push(json!({"network": "icmp", "action": "reject", "method": "drop"}))
            }
            "reply" => {
                route_rules.push(json!({"network": "icmp", "action": "reject", "method": "reply"}))
            }
            "rule" => {}
            other => {
                tracing::debug!("unknown tun_icmp '{other}'; leaving ICMP to normal routing")
            }
        }
    }
    route_rules.push(json!({"ip_is_private": true, "outbound": "direct"}));
    // Provider rules replace the user's preset and go before the user's custom rules.
    if let Some(provider) = provider {
        for rule in routing::provider_rules(provider) {
            route_rules.push(route_rule(&rule, &mut rule_sets, &geo));
        }
    }
    for rule in routing::custom_rules(&settings.routing) {
        route_rules.push(route_rule(&rule, &mut rule_sets, &geo));
    }
    if provider.is_none() {
        for rule in routing::preset_rules(&settings.routing) {
            route_rules.push(route_rule(&rule, &mut rule_sets, &geo));
        }
    }

    // A bootstrap resolver (any IP form) is used to resolve the DNS servers' own domains.
    let bootstrap_tag = (!bootstrap.is_empty()).then_some("bootstrap");
    let mut dns_servers = vec![
        dns_server(
            remote_dns,
            "remote",
            Some("proxy"),
            "1.1.1.1",
            bootstrap_tag,
        ),
        dns_server(direct_dns, "direct", None, "77.88.8.8", bootstrap_tag),
        json!({"type": "local", "tag": "local"}),
    ];
    if bootstrap_tag.is_some() {
        dns_servers.insert(
            0,
            json!({"type": "udp", "tag": "bootstrap", "server": bootstrap}),
        );
    }
    let mut dns_rules = Vec::new();
    if provider.is_none() && settings.routing.preset == RoutingPreset::BypassRu {
        rule_sets.insert("geosite-category-ru".into(), geo.geosite("category-ru"));
        dns_rules.push(json!({"rule_set": ["geosite-category-ru"], "server": "direct"}));
    }
    if fake_ip && tun {
        let range = fake_ip_range.trim();
        let range = if range.is_empty() {
            "198.18.0.0/15"
        } else {
            range
        };
        dns_servers.push(json!({
            "type": "fakeip",
            "tag": "fakeip",
            "inet4_range": range,
            "inet6_range": "fc00::/18",
        }));
        dns_rules.insert(0, json!({"query_type": ["A", "AAAA"], "server": "fakeip"}));
    }
    // Static hosts: the user `dns.hosts` first, then the provider `dns_hosts` override per key.
    let mut hosts_predefined: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (host, address) in &settings.dns.hosts {
        hosts_predefined.insert(host.clone(), vec![address.clone()]);
    }
    if let Some(provider) = provider {
        for (host, address) in &provider.dns_hosts {
            hosts_predefined.insert(host.clone(), vec![address.clone()]);
        }
    }
    if !hosts_predefined.is_empty() {
        let domains: Vec<String> = hosts_predefined.keys().cloned().collect();
        dns_servers.insert(
            0,
            json!({"type": "hosts", "tag": "hosts", "predefined": hosts_predefined}),
        );
        // Xray `dns.hosts` covers the domain *and its subdomains*; match that with
        // `domain_suffix` (sing-box `domain` would only match the exact name).
        dns_rules.insert(0, json!({"domain_suffix": domains, "server": "hosts"}));
    }
    // Block SVCB/HTTPS (type 64/65) so browsers fall back to A/AAAA; sing-box accepts a DNS
    // `reject` action (only in TUN the point is moot, but the rule is valid everywhere).
    if block_svcb {
        dns_rules.push(json!({"query_type": ["HTTPS", "SVCB"], "action": "reject"}));
    }

    let mut inbounds = Vec::new();
    if tun {
        let mut address = vec!["172.19.0.1/30".to_string()];
        if settings.tun_ipv6 {
            address.push("fdfe:dcba:9876::1/126".into());
        }
        // Always keep the private ranges excluded (loop prevention for the chained cores), then
        // append the user's `tun_route_exclude` CIDRs (de-duplicated, blanks skipped).
        let mut route_exclude: Vec<String> =
            PRIVATE_RANGES.iter().map(|cidr| cidr.to_string()).collect();
        for cidr in &settings.tun_route_exclude {
            let cidr = cidr.trim();
            if !cidr.is_empty() && !route_exclude.iter().any(|existing| existing == cidr) {
                route_exclude.push(cidr.to_string());
            }
        }
        inbounds.push(json!({
            "type": "tun",
            "tag": "tun-in",
            "address": address,
            "mtu": settings.tun_mtu,
            "stack": settings.tun_stack,
            "auto_route": settings.tun_auto_route,
            "strict_route": settings.tun_strict_route,
            "route_exclude_address": route_exclude,
        }));
    } else if settings.mode == crate::model::ConnectionMode::Tproxy && upstream_socks.is_none() {
        // Linux transparent-proxy capture: nftables TPROXY diverts matched packets to this
        // local port. No tun device is created, so applications see the real network. The
        // netfilter rules that make traffic reach this port live in flamix-sys (installed
        // alongside the elevated core, torn down on exit).
        inbounds.push(json!({
            "type": "tproxy",
            "tag": "tproxy-in",
            "listen": "127.0.0.1",
            "listen_port": settings.tproxy_port,
            "network": "tcp udp",
        }));
    } else if upstream_socks.is_none() {
        // With an upstream SOCKS chain the Xray process binds the local listener.
        // LIMITATION: sing-box's `mixed` inbound always serves UDP (TCP + UDP on one port);
        // there is no per-inbound UDP toggle, so `udp_enabled=false` only applies to the Xray
        // inbound. Noted for the user, never a config failure.
        if !settings.udp_enabled {
            tracing::debug!("settings.udp_enabled=false has no sing-box mixed equivalent; ignored");
        }
        let mut inbound = Map::new();
        inbound.insert("type".into(), json!("mixed"));
        inbound.insert("tag".into(), json!("mixed-in"));
        inbound.insert(
            "listen".into(),
            json!(if settings.allow_lan {
                "0.0.0.0"
            } else {
                "127.0.0.1"
            }),
        );
        inbound.insert("listen_port".into(), json!(settings.socks_port));
        let users = mixed_users(settings, ctx.overrides);
        if !users.is_empty() {
            inbound.insert("users".into(), json!(users));
        }
        inbounds.push(Value::Object(inbound));
    }

    let mut route = Map::new();
    route.insert("auto_detect_interface".into(), json!(true));
    // sing-box 1.12+: dialing a domain without a resolver is deprecated (removed in 1.14).
    route.insert(
        "default_domain_resolver".into(),
        json!({"server": "direct"}),
    );
    if !rule_sets.is_empty() {
        let sets: Vec<Value> = rule_sets
            .iter()
            .map(|(tag, url)| {
                json!({
                    "type": "remote",
                    "tag": tag,
                    "format": "binary",
                    "url": url,
                    "download_detour": "proxy",
                })
            })
            .collect();
        route.insert("rule_set".into(), json!(sets));
    }
    route.insert("rules".into(), json!(route_rules));
    // `global_proxy == false`: unmatched traffic goes direct instead of through the proxy.
    let final_outbound = match provider {
        Some(provider) if !provider.global_proxy => "direct",
        // The RF preset falls back to DIRECT; DNS keeps its own detour.
        None if routing::final_direct(&settings.routing) => "direct",
        _ => "proxy",
    };
    route.insert("final".into(), json!(final_outbound));

    let cluster = nodes.len() > 1;
    let mut outbounds = Vec::new();
    if !proxy_in_endpoints {
        if cluster {
            // `proxy` names the urltest balancer; the real outbounds are `node-N`.
            outbounds.push(urltest_outbound(nodes.len(), settings));
            for (i, node) in nodes.iter().copied().enumerate() {
                outbounds.push(node_outbound(node, settings, i));
            }
        } else {
            outbounds.push(proxy_out.clone());
        }
    }
    outbounds.push(json!({"type": "direct", "tag": "direct"}));

    let mut cfg = Map::new();
    cfg.insert(
        "log".into(),
        json!({"level": log_level(&settings.log_level)}),
    );
    cfg.insert(
        "dns".into(),
        json!({"servers": dns_servers, "rules": dns_rules}),
    );
    cfg.insert("inbounds".into(), json!(inbounds));
    if proxy_in_endpoints {
        cfg.insert("endpoints".into(), json!([proxy_out]));
    }
    cfg.insert("outbounds".into(), json!(outbounds));
    cfg.insert("route".into(), Value::Object(route));
    cfg.insert(
        "experimental".into(),
        json!({"clash_api": {"external_controller": format!("127.0.0.1:{api_port}")}}),
    );
    Value::Object(cfg)
}

/// Mixed-inbound `users` from the provider `socks-auth-*` / `http-auth-*` overrides (which win
/// per prefix) or, when neither is set, the user `local_auth_user`/`local_auth_pass`; de-duplicated.
fn mixed_users(settings: &Settings, overrides: Option<&BTreeMap<String, String>>) -> Vec<Value> {
    let mut users: Vec<Value> = Vec::new();
    for prefix in ["socks", "http"] {
        if let Some(auth) = effective_local_auth(prefix, settings, overrides) {
            let user = json!({"username": auth.user, "password": auth.password});
            if !users.contains(&user) {
                users.push(user);
            }
        }
    }
    users
}

/// Geo rule-set URLs for one config: the sing-box `.srs` bases from the settings
/// [`GeoData`](crate::model::GeoData) source, with a provider `.srs` URL (when present and
/// valid) overriding the base entirely.
struct GeoUrls {
    geosite: Option<String>,
    geoip: Option<String>,
    geosite_base: String,
    geoip_base: String,
}

impl GeoUrls {
    fn new(settings: &Settings, provider: Option<&ProviderRouting>) -> Self {
        let bases = crate::geo::urls(&settings.geo);
        Self {
            geosite: provider
                .and_then(|p| p.geosite_url.clone())
                .filter(|url| is_srs_url(url)),
            geoip: provider
                .and_then(|p| p.geoip_url.clone())
                .filter(|url| is_srs_url(url)),
            geosite_base: bases.singbox_geosite_base,
            geoip_base: bases.singbox_geoip_base,
        }
    }

    fn geosite(&self, name: &str) -> String {
        self.geosite
            .clone()
            .unwrap_or_else(|| geosite_url(&self.geosite_base, name))
    }

    fn geoip(&self, name: &str) -> String {
        self.geoip
            .clone()
            .unwrap_or_else(|| geoip_url(&self.geoip_base, name))
    }
}

fn non_empty(value: &Option<String>) -> Option<&str> {
    value.as_deref().filter(|s| !s.is_empty())
}

fn tls_json(security: &Security, settings: &Settings, required: bool) -> Option<Value> {
    match security {
        Security::None => {
            if required {
                Some(json!({"enabled": true}))
            } else {
                None
            }
        }
        Security::Tls {
            sni,
            alpn,
            fingerprint,
            allow_insecure,
        } => {
            let mut tls = Map::new();
            tls.insert("enabled".into(), json!(true));
            if let Some(sni) = non_empty(sni) {
                tls.insert("server_name".into(), json!(sni));
            }
            if !alpn.is_empty() {
                tls.insert("alpn".into(), json!(alpn));
            }
            if *allow_insecure {
                tls.insert("insecure".into(), json!(true));
            }
            if let Some(fp) = resolve_fingerprint(fingerprint, settings) {
                tls.insert("utls".into(), json!({"enabled": true, "fingerprint": fp}));
            }
            Some(Value::Object(tls))
        }
        Security::Reality {
            sni,
            public_key,
            short_id,
            fingerprint,
            ..
        } => {
            let mut reality = Map::new();
            reality.insert("enabled".into(), json!(true));
            reality.insert("public_key".into(), json!(public_key));
            if let Some(short_id) = non_empty(short_id) {
                reality.insert("short_id".into(), json!(short_id));
            }
            let mut tls = Map::new();
            tls.insert("enabled".into(), json!(true));
            if let Some(sni) = non_empty(sni) {
                tls.insert("server_name".into(), json!(sni));
            }
            // sing-box requires a uTLS fingerprint for a reality client.
            if let Some(fp) = resolve_fingerprint(fingerprint, settings) {
                tls.insert("utls".into(), json!({"enabled": true, "fingerprint": fp}));
            }
            tls.insert("reality".into(), Value::Object(reality));
            Some(Value::Object(tls))
        }
    }
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

fn transport_json(transport: &Transport) -> Option<Value> {
    match transport.network {
        Network::Tcp => None,
        Network::Ws => {
            let mut ws = Map::new();
            ws.insert("type".into(), json!("ws"));
            if let Some(path) = non_empty(&transport.path) {
                ws.insert("path".into(), json!(path));
            }
            if let Some(host) = non_empty(&transport.host) {
                ws.insert("headers".into(), json!({"Host": host}));
            }
            Some(Value::Object(ws))
        }
        Network::Grpc => {
            let mut grpc = Map::new();
            grpc.insert("type".into(), json!("grpc"));
            if let Some(service) = non_empty(&transport.service_name) {
                grpc.insert("service_name".into(), json!(service));
            }
            Some(Value::Object(grpc))
        }
        Network::HttpUpgrade => {
            let mut http = Map::new();
            http.insert("type".into(), json!("httpupgrade"));
            if let Some(host) = non_empty(&transport.host) {
                http.insert("host".into(), json!(host));
            }
            if let Some(path) = non_empty(&transport.path) {
                http.insert("path".into(), json!(path));
            }
            Some(Value::Object(http))
        }
        Network::H2 => {
            let mut http = Map::new();
            http.insert("type".into(), json!("http"));
            if let Some(host) = non_empty(&transport.host) {
                let hosts: Vec<&str> = host
                    .split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .collect();
                http.insert("host".into(), json!(hosts));
            }
            if let Some(path) = non_empty(&transport.path) {
                http.insert("path".into(), json!(path));
            }
            Some(Value::Object(http))
        }
        Network::Xhttp | Network::Kcp | Network::Quic => None,
    }
}

fn route_rule(rule: &Rule, sets: &mut BTreeMap<String, String>, geo: &GeoUrls) -> Value {
    let mut m = Map::new();
    let mut rule_sets = Vec::new();
    let mut domain = Vec::new();
    let mut domain_suffix = Vec::new();
    let mut domain_regex = Vec::new();
    let mut domain_keyword = Vec::new();
    for value in &rule.domains {
        if let Some(name) = value.strip_prefix("geosite:") {
            let tag = format!("geosite-{name}");
            sets.insert(tag.clone(), geo.geosite(name));
            rule_sets.push(tag);
        } else if let Some(value) = value.strip_prefix("domain:") {
            domain_suffix.push(value.to_string());
        } else if let Some(value) = value.strip_prefix("keyword:") {
            domain_keyword.push(value.to_string());
        } else if let Some(value) = value.strip_prefix("full:") {
            domain.push(value.to_string());
        } else if let Some(value) = value.strip_prefix("regexp:") {
            domain_regex.push(value.to_string());
        } else {
            domain_suffix.push(value.clone());
        }
    }
    let mut ip_cidr = Vec::new();
    let mut ip_private = false;
    for value in &rule.ips {
        if value == "geoip:private" {
            ip_private = true;
        } else if let Some(name) = value.strip_prefix("geoip:") {
            let tag = format!("geoip-{name}");
            sets.insert(tag.clone(), geo.geoip(name));
            rule_sets.push(tag);
        } else {
            ip_cidr.push(value.clone());
        }
    }

    if !domain.is_empty() {
        m.insert("domain".into(), json!(domain));
    }
    if !domain_suffix.is_empty() {
        m.insert("domain_suffix".into(), json!(domain_suffix));
    }
    if !domain_regex.is_empty() {
        m.insert("domain_regex".into(), json!(domain_regex));
    }
    if !domain_keyword.is_empty() {
        m.insert("domain_keyword".into(), json!(domain_keyword));
    }
    if !rule_sets.is_empty() {
        m.insert("rule_set".into(), json!(rule_sets));
    }
    if ip_private {
        m.insert("ip_is_private".into(), json!(true));
    }
    if !ip_cidr.is_empty() {
        m.insert("ip_cidr".into(), json!(ip_cidr));
    }
    if !rule.processes.is_empty() {
        m.insert("process_name".into(), json!(rule.processes));
    }
    if let Some(ports) = non_empty(&rule.ports) {
        let (list, ranges) = parse_ports(ports);
        if !list.is_empty() {
            m.insert("port".into(), json!(list));
        }
        if !ranges.is_empty() {
            m.insert("port_range".into(), json!(ranges));
        }
    }
    match rule.action {
        RuleAction::Block => {
            m.insert("action".into(), json!("reject"));
        }
        RuleAction::Proxy => {
            m.insert("outbound".into(), json!("proxy"));
        }
        RuleAction::Direct => {
            m.insert("outbound".into(), json!("direct"));
        }
    }
    Value::Object(m)
}

/// True when `url` points at a sing-box rule-set (`.srs`, case-insensitive, ignoring any
/// query string or fragment). A `.dat` geo file (Xray format) is not a valid rule-set.
fn is_srs_url(url: &str) -> bool {
    let path = url
        .split(['?', '#'])
        .next()
        .unwrap_or(url)
        .trim_end_matches('/');
    path.to_ascii_lowercase().ends_with(".srs")
}

fn geosite_url(base: &str, name: &str) -> String {
    format!("{base}/geosite-{name}.srs")
}

fn geoip_url(base: &str, name: &str) -> String {
    format!("{base}/geoip-{name}.srs")
}

/// "20000-30000" / "20000-30000,40000" -> ["20000:30000", "40000"].
fn parse_server_ports(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|part| match part.split_once('-') {
            Some((start, end)) => format!("{}:{}", start.trim(), end.trim()),
            None => part.to_string(),
        })
        .collect()
}

fn parse_ports(value: &str) -> (Vec<u16>, Vec<String>) {
    let mut ports = Vec::new();
    let mut ranges = Vec::new();
    for part in value.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        if let Some((start, end)) = part.split_once('-') {
            if let (Ok(start), Ok(end)) = (start.trim().parse::<u16>(), end.trim().parse::<u16>()) {
                ranges.push(format!("{start}:{end}"));
            }
        } else if let Ok(port) = part.parse::<u16>() {
            ports.push(port);
        }
    }
    (ports, ranges)
}

/// Parse an upstream DNS URL into the sing-box 1.12+ `{type, server, tag}` shape.
///
/// `detour` is the outbound tag used to reach the server (`"proxy"` for the remote
/// resolver, `None` for the direct one). A server given by domain name gets an explicit
/// `domain_resolver` pointing at `bootstrap` when set, else the `local` (system) resolver.
fn dns_server(
    raw: &str,
    tag: &str,
    detour: Option<&str>,
    fallback: &str,
    bootstrap: Option<&str>,
) -> Value {
    let mut m = Map::new();
    if raw == "localhost" || raw == "local" {
        m.insert("type".into(), json!("local"));
        m.insert("tag".into(), json!(tag));
        return Value::Object(m);
    }
    let (kind, host, port, path) = match url::Url::parse(raw) {
        Ok(url) => {
            let kind: String = match url.scheme() {
                "http" | "https" => "https".into(),
                "tls" => "tls".into(),
                "quic" => "quic".into(),
                "h3" => "h3".into(),
                "tcp" => "tcp".into(),
                "udp" => "udp".into(),
                other => other.to_string(),
            };
            let path = if kind == "https" && !url.path().is_empty() && url.path() != "/" {
                Some(url.path().to_string())
            } else {
                None
            };
            (
                kind,
                url.host_str().unwrap_or(fallback).to_string(),
                url.port(),
                path,
            )
        }
        Err(_) => ("udp".to_string(), raw.to_string(), None, None),
    };
    m.insert("type".into(), json!(kind));
    m.insert("tag".into(), json!(tag));
    m.insert("server".into(), json!(host));
    if let Some(port) = port {
        m.insert("server_port".into(), json!(port));
    }
    if let Some(path) = path {
        m.insert("path".into(), json!(path));
    }
    if let Some(detour) = detour {
        m.insert("detour".into(), json!(detour));
    }
    if is_domain(&host) {
        m.insert(
            "domain_resolver".into(),
            json!(bootstrap.unwrap_or("local")),
        );
    }
    Value::Object(m)
}

/// Xray log levels use "warning"; sing-box uses "warn".
fn log_level(level: &str) -> String {
    match level.to_ascii_lowercase().as_str() {
        "warning" => "warn".into(),
        "none" | "silent" => "fatal".into(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ConnectionMode, Dns, Routing, RoutingPreset};

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

    #[test]
    fn hysteria2_hopping() {
        let mut s = server(Credentials::Hysteria2 {
            password: "pw".into(),
            obfs: Some("salamander".into()),
            obfs_password: Some("op".into()),
            ports: Some("20000-30000".into()),
            up_mbps: Some(100),
            down_mbps: Some(200),
        });
        s.security = Security::Tls {
            sni: Some("hy.example".into()),
            alpn: vec!["h3".into()],
            fingerprint: None,
            allow_insecure: false,
        };
        let out = outbound(&s, &Settings::default()).unwrap();
        assert_eq!(out["type"], json!("hysteria2"));
        assert_eq!(out["server"], json!("example.com"));
        assert_eq!(out["server_port"], json!(443));
        assert_eq!(out["password"], json!("pw"));
        assert_eq!(out["obfs"]["type"], json!("salamander"));
        assert_eq!(out["obfs"]["password"], json!("op"));
        assert_eq!(out["server_ports"], json!(["20000:30000"]));
        assert_eq!(out["up_mbps"], json!(100));
        assert_eq!(out["down_mbps"], json!(200));
        assert_eq!(out["tls"]["enabled"], json!(true));
        assert_eq!(out["tls"]["server_name"], json!("hy.example"));
        assert_eq!(out["tls"]["alpn"], json!(["h3"]));
    }

    #[test]
    fn tuic_outbound() {
        let s = server(Credentials::Tuic {
            uuid: "33333333-3333-3333-3333-333333333333".into(),
            password: "secret".into(),
            congestion_control: Some("bbr".into()),
            udp_relay_mode: Some("native".into()),
        });
        let out = outbound(&s, &Settings::default()).unwrap();
        assert_eq!(out["type"], json!("tuic"));
        assert_eq!(out["uuid"], json!("33333333-3333-3333-3333-333333333333"));
        assert_eq!(out["password"], json!("secret"));
        assert_eq!(out["congestion_control"], json!("bbr"));
        assert_eq!(out["udp_relay_mode"], json!("native"));
        assert_eq!(out["tls"]["enabled"], json!(true));
    }

    #[test]
    fn wireguard_is_an_endpoint() {
        let s = server(Credentials::Wireguard {
            private_key: "priv".into(),
            peer_public_key: "pub".into(),
            pre_shared_key: Some("psk".into()),
            local_address: vec!["10.0.0.2/32".into()],
            reserved: Some([1, 2, 3]),
            mtu: Some(1420),
        });
        let out = outbound(&s, &Settings::default()).unwrap();
        assert_eq!(out["type"], json!("wireguard"));
        assert_eq!(out["tag"], json!("proxy"));
        assert_eq!(out["private_key"], json!("priv"));
        assert_eq!(out["peers"][0]["public_key"], json!("pub"));
        assert_eq!(out["peers"][0]["pre_shared_key"], json!("psk"));
        assert_eq!(out["peers"][0]["address"], json!("example.com"));
        assert_eq!(out["peers"][0]["port"], json!(443));
        assert_eq!(out["peers"][0]["allowed_ips"], json!(["0.0.0.0/0", "::/0"]));
        assert_eq!(out["peers"][0]["reserved"], json!([1, 2, 3]));
        assert_eq!(out["mtu"], json!(1420));

        // ... and it must land in `endpoints`, not `outbounds`.
        let cfg = full_config(&s, &Settings::default(), false, None, 10819);
        assert_eq!(cfg["endpoints"][0]["type"], json!("wireguard"));
        assert_eq!(cfg["outbounds"][0]["tag"], json!("direct"));
        assert!(cfg.get("endpoints").is_some());
    }

    #[test]
    fn xhttp_and_kcp_unsupported() {
        let mut s = server(Credentials::Vless {
            uuid: "u".into(),
            flow: None,
            encryption: None,
        });
        s.transport = Transport {
            network: Network::Xhttp,
            ..Transport::default()
        };
        assert!(outbound(&s, &Settings::default()).is_none());
        s.transport.network = Network::Kcp;
        assert!(outbound(&s, &Settings::default()).is_none());
    }

    #[test]
    fn vless_reality_outbound() {
        let mut s = server(Credentials::Vless {
            uuid: "u".into(),
            flow: Some("xtls-rprx-vision".into()),
            encryption: None,
        });
        s.security = Security::Reality {
            sni: Some("r.example".into()),
            public_key: "PUB".into(),
            short_id: Some("sid".into()),
            spider_x: None,
            fingerprint: None,
        };
        let out = outbound(&s, &Settings::default()).unwrap();
        assert_eq!(out["flow"], json!("xtls-rprx-vision"));
        assert_eq!(out["tls"]["enabled"], json!(true));
        assert_eq!(out["tls"]["server_name"], json!("r.example"));
        assert_eq!(out["tls"]["reality"]["enabled"], json!(true));
        assert_eq!(out["tls"]["reality"]["public_key"], json!("PUB"));
        assert_eq!(out["tls"]["reality"]["short_id"], json!("sid"));
    }

    #[test]
    fn full_config_proxy_shape() {
        let s = server(Credentials::Vless {
            uuid: "u".into(),
            flow: None,
            encryption: None,
        });
        let settings = Settings {
            routing: Routing {
                preset: RoutingPreset::BypassRu,
                ..Routing::default()
            },
            mode: ConnectionMode::Proxy,
            dns: Dns::default(),
            ..Settings::default()
        };
        let cfg = full_config(&s, &settings, false, None, 10819);
        assert_eq!(cfg["log"]["level"], json!("warn"));
        assert_eq!(cfg["inbounds"][0]["type"], json!("mixed"));
        assert_eq!(cfg["inbounds"][0]["listen_port"], json!(10808));
        assert_eq!(cfg["outbounds"][0]["tag"], json!("proxy"));
        assert_eq!(cfg["outbounds"][1]["tag"], json!("direct"));

        let rules = cfg["route"]["rules"].as_array().unwrap();
        assert_eq!(
            rules[0],
            json!({"action": "sniff", "sniffer": ["http", "tls", "quic"]})
        );
        assert_eq!(rules[1], json!({"protocol": "dns", "action": "hijack-dns"}));
        assert!(rules
            .iter()
            .any(|r| r["ip_is_private"] == json!(true) && r["outbound"] == json!("direct")));
        // RF preset: blocked services via a geosite rule set to the proxy, fallback DIRECT.
        assert!(rules.iter().any(|r| r["rule_set"]
            == json!(["geosite-ru-blocked", "geosite-ru-blocked-all"])
            && r["outbound"] == json!("proxy")));
        assert!(rules.iter().any(|r| r["domain_keyword"].is_array()));
        assert!(rules
            .iter()
            .any(|r| r["process_name"].is_array() && r["outbound"] == json!("proxy")));
        assert_eq!(cfg["route"]["final"], json!("direct"));
        assert_eq!(cfg["route"]["auto_detect_interface"], json!(true));

        assert_eq!(
            cfg["experimental"]["clash_api"]["external_controller"],
            json!("127.0.0.1:10819")
        );
        assert_eq!(cfg["dns"]["servers"][0]["tag"], json!("remote"));
        assert_eq!(cfg["dns"]["servers"][0]["type"], json!("https"));
        assert_eq!(cfg["dns"]["servers"][0]["server"], json!("1.1.1.1"));
        assert_eq!(cfg["dns"]["servers"][0]["detour"], json!("proxy"));
        assert_eq!(cfg["dns"]["servers"][1]["tag"], json!("direct"));
        assert!(cfg["dns"]["servers"][1].get("detour").is_none());
        assert_eq!(
            cfg["dns"]["servers"][2],
            json!({"type": "local", "tag": "local"})
        );
        assert_eq!(
            cfg["route"]["default_domain_resolver"],
            json!({"server": "direct"})
        );

        let sets = cfg["route"]["rule_set"].as_array().unwrap();
        assert!(sets.iter().any(|s| {
            s["tag"] == json!("geosite-category-ru") && s["download_detour"] == json!("proxy")
        }));
    }

    #[test]
    fn full_config_tproxy_shape() {
        let s = server(Credentials::Vless {
            uuid: "u".into(),
            flow: None,
            encryption: None,
        });
        let settings = Settings {
            mode: ConnectionMode::Tproxy,
            tproxy_port: 24680,
            ..Settings::default()
        };
        // TProxy uses the sing-box native outbound path (no upstream SOCKS, no tun).
        let cfg = full_config(&s, &settings, false, None, 10819);

        let inbound = &cfg["inbounds"][0];
        assert_eq!(inbound["type"], json!("tproxy"));
        assert_eq!(inbound["listen"], json!("127.0.0.1"));
        assert_eq!(inbound["listen_port"], json!(24680));
        assert_eq!(inbound["network"], json!("tcp udp"));
        // No mixed and no tun inbound in TProxy mode.
        let inbounds = cfg["inbounds"].as_array().unwrap();
        assert!(inbounds
            .iter()
            .all(|i| i["type"] != json!("mixed") && i["type"] != json!("tun")));
    }

    #[test]
    fn full_config_tun_shape() {
        let mut s = server(Credentials::Vless {
            uuid: "u".into(),
            flow: None,
            encryption: None,
        });
        s.transport = Transport {
            network: Network::Ws,
            path: Some("/ws".into()),
            ..Transport::default()
        };
        let settings = Settings {
            mode: ConnectionMode::Tun,
            tun_ipv6: true,
            dns: Dns {
                fake_ip: true,
                ..Dns::default()
            },
            ..Settings::default()
        };
        let cfg = full_config(&s, &settings, true, None, 10819);

        assert_eq!(cfg["inbounds"][0]["type"], json!("tun"));
        assert_eq!(
            cfg["inbounds"][0]["address"],
            json!(["172.19.0.1/30", "fdfe:dcba:9876::1/126"])
        );
        assert_eq!(cfg["inbounds"][0]["auto_route"], json!(true));
        assert_eq!(cfg["inbounds"][0]["strict_route"], json!(true));
        assert!(cfg["inbounds"][0]["route_exclude_address"]
            .as_array()
            .unwrap()
            .contains(&json!("192.168.0.0/16")));
        // No mixed inbound in TUN mode.
        assert!(cfg["inbounds"]
            .as_array()
            .unwrap()
            .iter()
            .all(|i| i["type"] != json!("mixed")));

        let rules = cfg["route"]["rules"].as_array().unwrap();
        assert!(rules.iter().any(|r| {
            r["process_name"] == json!(["xray", "xray.exe", "sing-box", "sing-box.exe"])
                && r["outbound"] == json!("direct")
        }));
        assert_eq!(cfg["dns"]["rules"][0]["server"], json!("fakeip"));
        assert!(cfg["dns"]["servers"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["type"] == json!("fakeip")));
    }

    #[test]
    fn upstream_socks_chain_config() {
        let mut s = server(Credentials::Vless {
            uuid: "u".into(),
            flow: None,
            encryption: None,
        });
        s.transport = Transport {
            network: Network::Xhttp,
            ..Transport::default()
        };
        let cfg = full_config(&s, &Settings::default(), true, Some(10808), 10819);
        assert_eq!(cfg["outbounds"][0]["type"], json!("socks"));
        assert_eq!(cfg["outbounds"][0]["tag"], json!("proxy"));
        assert_eq!(cfg["outbounds"][0]["server"], json!("127.0.0.1"));
        assert_eq!(cfg["outbounds"][0]["server_port"], json!(10808));
        // The Xray process owns the local listener; sing-box must not bind mixed.
        assert!(cfg["inbounds"]
            .as_array()
            .unwrap()
            .iter()
            .all(|i| i["type"] != json!("mixed")));
    }

    #[test]
    fn global_preset_private_direct() {
        let s = server(Credentials::Trojan {
            password: "p".into(),
        });
        let mut settings = Settings::default();
        settings.routing.preset = RoutingPreset::Global;
        let cfg = full_config(&s, &settings, false, None, 10819);
        let rules = cfg["route"]["rules"].as_array().unwrap();
        assert!(rules.iter().any(|r| r["ip_is_private"] == json!(true)));
        assert_eq!(cfg["route"]["final"], json!("proxy"));
    }

    use crate::model::RoutingRule;

    fn provider_profile(global_proxy: bool, rules: Vec<RoutingRule>) -> ProviderRouting {
        ProviderRouting {
            name: "provider".into(),
            enabled: true,
            global_proxy,
            rules,
            ..ProviderRouting::default()
        }
    }

    fn rule(domains: Vec<&str>, ips: Vec<&str>, action: RuleAction) -> RoutingRule {
        RoutingRule {
            domains: domains.into_iter().map(str::to_string).collect(),
            ips: ips.into_iter().map(str::to_string).collect(),
            processes: Vec::new(),
            network: None,
            ports: None,
            action,
            enabled: true,
        }
    }

    fn vless() -> Server {
        server(Credentials::Vless {
            uuid: "u".into(),
            flow: None,
            encryption: None,
        })
    }

    #[test]
    fn provider_rules_replace_preset_and_come_before_custom() {
        let mut settings = Settings::default();
        settings.routing.preset = RoutingPreset::BypassRu;
        settings.routing.custom_rules = vec![rule(
            vec!["domain:custom.example"],
            Vec::new(),
            RuleAction::Direct,
        )];
        let provider = provider_profile(
            true,
            vec![rule(vec!["geosite:google"], Vec::new(), RuleAction::Proxy)],
        );
        let ctx = ConnectContext {
            provider: Some(&provider),
            overrides: None,
        };
        let cfg = full_config_with(&vless(), &settings, false, None, 10819, &ctx);
        let rules = cfg["route"]["rules"].as_array().unwrap();
        let google = rules
            .iter()
            .position(|r| r["rule_set"] == json!(["geosite-google"]))
            .expect("provider rule");
        let custom = rules
            .iter()
            .position(|r| r["domain_suffix"] == json!(["custom.example"]))
            .expect("custom rule");
        assert!(google < custom, "provider rules go before custom rules");
        // BypassRu preset must be gone.
        assert!(!rules
            .iter()
            .any(|r| r["rule_set"] == json!(["geosite-category-ads-all"])));
        assert_eq!(cfg["route"]["final"], json!("proxy"));
    }

    #[test]
    fn provider_global_proxy_false_final_direct() {
        let provider = provider_profile(false, Vec::new());
        let ctx = ConnectContext {
            provider: Some(&provider),
            overrides: None,
        };
        let cfg = full_config_with(&vless(), &Settings::default(), false, None, 10819, &ctx);
        assert_eq!(cfg["route"]["final"], json!("direct"));

        let provider = provider_profile(true, Vec::new());
        let ctx = ConnectContext {
            provider: Some(&provider),
            overrides: None,
        };
        let cfg = full_config_with(&vless(), &Settings::default(), false, None, 10819, &ctx);
        assert_eq!(cfg["route"]["final"], json!("proxy"));
    }

    #[test]
    fn provider_dns_and_hosts_override() {
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
        let ctx = ConnectContext {
            provider: Some(&provider),
            overrides: None,
        };
        let cfg = full_config_with(&vless(), &Settings::default(), false, None, 10819, &ctx);

        let servers = cfg["dns"]["servers"].as_array().unwrap();
        assert_eq!(servers[0]["type"], json!("hosts"));
        assert_eq!(servers[0]["predefined"]["example.com"], json!(["10.0.0.1"]));
        let remote = servers
            .iter()
            .find(|s| s["tag"] == json!("remote"))
            .unwrap();
        assert_eq!(remote["server"], json!("9.9.9.9"));
        let direct = servers
            .iter()
            .find(|s| s["tag"] == json!("direct"))
            .unwrap();
        assert_eq!(direct["server"], json!("8.8.8.8"));
        assert_eq!(cfg["dns"]["rules"][0]["server"], json!("hosts"));
        assert_eq!(
            cfg["dns"]["rules"][0]["domain_suffix"],
            json!(["example.com"])
        );
    }

    #[test]
    fn provider_fake_dns_enables_fakeip_in_tun() {
        let mut provider = provider_profile(true, Vec::new());
        provider.fake_dns = true;
        let ctx = ConnectContext {
            provider: Some(&provider),
            overrides: None,
        };
        let cfg = full_config_with(&vless(), &Settings::default(), true, None, 10819, &ctx);
        assert!(cfg["dns"]["servers"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["type"] == json!("fakeip")));
    }

    #[test]
    fn provider_geo_urls_used_only_for_srs() {
        let mut provider = provider_profile(
            true,
            vec![rule(
                vec!["geosite:google"],
                vec!["geoip:ru"],
                RuleAction::Proxy,
            )],
        );
        // Upper-case suffix plus query/fragment still counts as a rule-set URL.
        provider.geosite_url = Some("https://cdn.example/GEOSITE.SRS?v=2#frag".into());
        provider.geoip_url = Some("https://cdn.example/geoip.dat".into());
        let ctx = ConnectContext {
            provider: Some(&provider),
            overrides: None,
        };
        let cfg = full_config_with(&vless(), &Settings::default(), false, None, 10819, &ctx);
        let sets = cfg["route"]["rule_set"].as_array().unwrap();
        assert!(sets.iter().any(|s| s["tag"] == json!("geosite-google")
            && s["url"] == json!("https://cdn.example/GEOSITE.SRS?v=2#frag")));
        // The .dat geoip URL is not a sing-box rule-set: keep the settings source's .srs base
        // (the default "Freedom for Russia" lists).
        let geoip = sets.iter().find(|s| s["tag"] == json!("geoip-ru")).unwrap();
        assert!(geoip["url"].as_str().unwrap().ends_with("geoip-ru.srs"));
        assert!(geoip["url"]
            .as_str()
            .unwrap()
            .contains("runetfreedom/russia-v2ray-rules-dat"));
    }

    #[test]
    fn override_enables_mixed_auth() {
        let overrides = BTreeMap::from([
            ("socks-auth-mode".to_string(), "manual".to_string()),
            ("socks-auth-user".to_string(), "u".to_string()),
            ("socks-auth-password".to_string(), "p".to_string()),
        ]);
        let ctx = ConnectContext {
            provider: None,
            overrides: Some(&overrides),
        };
        let cfg = full_config_with(&vless(), &Settings::default(), false, None, 10819, &ctx);
        let inbound = &cfg["inbounds"][0];
        assert_eq!(inbound["type"], json!("mixed"));
        assert_eq!(inbound["users"][0]["username"], json!("u"));
        assert_eq!(inbound["users"][0]["password"], json!("p"));
    }

    #[test]
    fn mixed_inbound_sniffing_and_sniffers() {
        // Default: sniff with the dest_override list.
        let cfg = full_config(&vless(), &Settings::default(), false, None, 10819);
        let rules = cfg["route"]["rules"].as_array().unwrap();
        assert_eq!(
            rules[0],
            json!({"action": "sniff", "sniffer": ["http", "tls", "quic"]})
        );

        // Sniffing off: no `sniff` action at all.
        let off = Settings {
            sniffing: false,
            ..Settings::default()
        };
        let cfg = full_config(&vless(), &off, false, None, 10819);
        let rules = cfg["route"]["rules"].as_array().unwrap();
        assert!(!rules.iter().any(|r| r["action"] == json!("sniff")));

        // `fakedns` has no sing-box sniffer: it is dropped and, with nothing left, the
        // `sniffer` field is omitted (sniff everything).
        let fake = Settings {
            dest_override: vec!["fakedns".into()],
            ..Settings::default()
        };
        let cfg = full_config(&vless(), &fake, false, None, 10819);
        let rules = cfg["route"]["rules"].as_array().unwrap();
        assert_eq!(rules[0], json!({"action": "sniff"}));
    }

    #[test]
    fn tun_auto_route_strict_route_and_exclude() {
        let settings = Settings {
            mode: ConnectionMode::Tun,
            tun_auto_route: false,
            tun_strict_route: true,
            tun_route_exclude: vec!["203.0.113.0/24".into(), "  10.0.0.0/8  ".into()],
            ..Settings::default()
        };
        let cfg = full_config(&vless(), &settings, true, None, 10819);
        let tun = &cfg["inbounds"][0];
        assert_eq!(tun["auto_route"], json!(false));
        assert_eq!(tun["strict_route"], json!(true));
        let excludes: Vec<&str> = tun["route_exclude_address"]
            .as_array()
            .unwrap()
            .iter()
            .map(|value| value.as_str().unwrap())
            .collect();
        assert!(excludes.contains(&"203.0.113.0/24"));
        // The user's private CIDR is de-duplicated against the built-in list.
        assert_eq!(
            excludes
                .iter()
                .filter(|cidr| **cidr == "10.0.0.0/8")
                .count(),
            1
        );
    }

    #[test]
    fn tun_icmp_variants_map_to_route_rules() {
        let cases = [
            ("direct", json!({"network": "icmp", "outbound": "direct"})),
            (
                "unreachable",
                json!({"network": "icmp", "action": "reject"}),
            ),
            (
                "drop",
                json!({"network": "icmp", "action": "reject", "method": "drop"}),
            ),
            (
                "reply",
                json!({"network": "icmp", "action": "reject", "method": "reply"}),
            ),
        ];
        for (value, expected) in cases {
            let settings = Settings {
                mode: ConnectionMode::Tun,
                tun_icmp: value.into(),
                ..Settings::default()
            };
            let cfg = full_config(&vless(), &settings, true, None, 10819);
            let rules = cfg["route"]["rules"].as_array().unwrap();
            assert!(rules.contains(&expected), "value={value}");
        }

        // "rule" leaves ICMP to normal routing: no ICMP-specific rule.
        let settings = Settings {
            mode: ConnectionMode::Tun,
            tun_icmp: "rule".into(),
            ..Settings::default()
        };
        let cfg = full_config(&vless(), &settings, true, None, 10819);
        let rules = cfg["route"]["rules"].as_array().unwrap();
        assert!(!rules.iter().any(|rule| rule["network"] == json!("icmp")));
    }

    #[test]
    fn dns_fakeip_range_bootstrap_block_svcb_and_hosts() {
        let settings = Settings {
            mode: ConnectionMode::Tun,
            dns: Dns {
                remote: "https://dns.google/dns-query".into(),
                fake_ip: true,
                fake_ip_range: "198.19.0.0/16".into(),
                bootstrap: "77.88.8.8".into(),
                block_svcb: true,
                hosts: BTreeMap::from([("example.com".into(), "10.0.0.1".into())]),
                ..Dns::default()
            },
            ..Settings::default()
        };
        let cfg = full_config(&vless(), &settings, true, None, 10819);
        let servers = cfg["dns"]["servers"].as_array().unwrap();

        let bootstrap = servers
            .iter()
            .find(|s| s["tag"] == json!("bootstrap"))
            .expect("bootstrap server");
        assert_eq!(bootstrap["type"], json!("udp"));
        assert_eq!(bootstrap["server"], json!("77.88.8.8"));
        let remote = servers
            .iter()
            .find(|s| s["tag"] == json!("remote"))
            .unwrap();
        assert_eq!(remote["domain_resolver"], json!("bootstrap"));

        let fakeip = servers
            .iter()
            .find(|s| s["type"] == json!("fakeip"))
            .unwrap();
        assert_eq!(fakeip["inet4_range"], json!("198.19.0.0/16"));

        let hosts = servers
            .iter()
            .find(|s| s["type"] == json!("hosts"))
            .unwrap();
        assert_eq!(hosts["predefined"]["example.com"], json!(["10.0.0.1"]));

        let rules = cfg["dns"]["rules"].as_array().unwrap();
        assert!(rules
            .iter()
            .any(|rule| rule["query_type"] == json!(["HTTPS", "SVCB"])
                && rule["action"] == json!("reject")));
    }

    #[test]
    fn outbound_mux_wired_for_supported_protocols() {
        let on = outbound(
            &vless(),
            &Settings {
                mux: true,
                mux_concurrency: 16,
                ..Settings::default()
            },
        )
        .unwrap();
        assert_eq!(on["multiplex"]["enabled"], json!(true));
        assert_eq!(on["multiplex"]["max_streams"], json!(16));

        // Off by default.
        let off = outbound(&vless(), &Settings::default()).unwrap();
        assert!(off.get("multiplex").is_none());

        // A vless xtls flow disables mux (mirrors Xray).
        let flow = server(Credentials::Vless {
            uuid: "u".into(),
            flow: Some("xtls-rprx-vision".into()),
            encryption: None,
        });
        let out = outbound(
            &flow,
            &Settings {
                mux: true,
                ..Settings::default()
            },
        )
        .unwrap();
        assert!(out.get("multiplex").is_none());

        // hysteria2 manages its own multiplexing.
        let hy = server(Credentials::Hysteria2 {
            password: "p".into(),
            obfs: None,
            obfs_password: None,
            ports: None,
            up_mbps: None,
            down_mbps: None,
        });
        let out = outbound(
            &hy,
            &Settings {
                mux: true,
                ..Settings::default()
            },
        )
        .unwrap();
        assert!(out.get("multiplex").is_none());
    }
}

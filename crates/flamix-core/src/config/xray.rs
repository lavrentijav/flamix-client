//! Xray-core JSON config. OWNER: task T03.
//!
//! Targets current Xray-core (v25/v26): the `raw` network replaces `tcp` (with
//! `rawSettings.header`), `xhttpSettings` carries the xhttp mode/extra, and the
//! DNS `servers` array takes `{address, domains}` objects.

use crate::config::routing::{self, Rule};
use crate::model::{Credentials, Network, RuleAction, Security, Server, Settings, Transport};
use serde_json::{json, Map, Value};

/// Outbound object (`"tag": "proxy"`) for `server`.
pub fn outbound(server: &Server, settings: &Settings) -> Value {
    let mut obj = Map::new();
    obj.insert("tag".into(), json!("proxy"));

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
    let listen = if settings.allow_lan {
        "0.0.0.0"
    } else {
        "127.0.0.1"
    };
    json!({
        "log": {"loglevel": settings.log_level},
        "api": {"tag": "api", "services": ["StatsService"]},
        "stats": {},
        "policy": {"system": {
            "statsInboundUplink": true,
            "statsInboundDownlink": true,
            "statsOutboundUplink": true,
            "statsOutboundDownlink": true,
        }},
        "dns": dns_config(settings),
        "inbounds": [
            {
                "tag": "socks",
                "listen": listen,
                "port": settings.socks_port,
                "protocol": "socks",
                "settings": {"auth": "noauth", "udp": true},
                "sniffing": {
                    "enabled": true,
                    "destOverride": ["http", "tls", "quic"],
                    "routeOnly": true,
                },
            },
            {
                "tag": "http",
                "listen": listen,
                "port": settings.http_port,
                "protocol": "http",
                "settings": {},
            },
            {
                "tag": "api",
                "listen": "127.0.0.1",
                "port": api_port,
                "protocol": "dokodemo-door",
                "settings": {"address": "127.0.0.1"},
            },
        ],
        "outbounds": [
            outbound(server, settings),
            {"protocol": "freedom", "tag": "direct"},
            {"protocol": "blackhole", "tag": "block"},
        ],
        "routing": routing_config(settings),
    })
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

fn dns_config(settings: &Settings) -> Value {
    let direct = if settings.routing.preset == crate::model::RoutingPreset::BypassRu {
        json!({"address": settings.dns.direct, "domains": ["geosite:category-ru"]})
    } else {
        json!(settings.dns.direct)
    };
    json!({"servers": [settings.dns.remote, direct]})
}

fn routing_config(settings: &Settings) -> Value {
    let mut rules = vec![json!({
        "type": "field",
        "inboundTag": ["api"],
        "outboundTag": "api",
    })];
    for rule in routing::custom_rules(&settings.routing) {
        rules.push(rule_json(&rule));
    }
    if settings.routing.block_quic {
        rules.push(json!({
            "type": "field",
            "network": "udp",
            "port": "443",
            "outboundTag": "block",
        }));
    }
    for rule in routing::preset_rules(settings.routing.preset) {
        rules.push(rule_json(&rule));
    }
    json!({
        "domainStrategy": settings.routing.domain_strategy,
        "rules": rules,
    })
}

fn rule_json(rule: &Rule) -> Value {
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
    // Process-based rules are ignored by Xray.
    m.insert(
        "outboundTag".into(),
        json!(match rule.action {
            RuleAction::Proxy => "proxy",
            RuleAction::Direct => "direct",
            RuleAction::Block => "block",
        }),
    );
    Value::Object(m)
}

/// `None`/empty -> no field.
fn non_empty(value: &Option<String>) -> Option<&str> {
    value.as_deref().filter(|s| !s.is_empty())
}

fn add_mux(obj: &mut Map<String, Value>, settings: &Settings, allow: bool) {
    if settings.mux && allow {
        obj.insert("mux".into(), json!({"enabled": true}));
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
        // api rule, block-quic rule, ads block, direct domains, direct ips.
        assert!(rules.iter().any(|r| r["network"] == json!("udp")
            && r["port"] == json!("443")
            && r["outboundTag"] == json!("block")));
        assert!(rules
            .iter()
            .any(|r| r["domain"] == json!(["geosite:category-ads-all"])
                && r["outboundTag"] == json!("block")));
        assert!(rules.iter().any(|r| r["outboundTag"] == json!("direct")
            && r["ip"] == json!(["geoip:private", "geoip:ru"])));
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
}

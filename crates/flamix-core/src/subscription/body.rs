//! Subscription body decoding: base64 / plain link lists, Xray & sing-box JSON, Clash YAML.

use super::util::decode_b64_str;
use crate::link;
use crate::model::{Credentials, Network, Security, Server, TrafficInfo, Transport};
use crate::{Error, Result};
use serde_json::Value;

/// Decode a subscription body into share links text (one link per line):
/// base64 (std/url-safe, padded or not), plain text, or JSON / Clash YAML (best effort).
pub fn decode_body(body: &str) -> Result<String> {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return Err(Error::Parse("empty subscription body".to_string()));
    }

    // 1. base64 of a newline separated share-link list.
    if let Some(decoded) = decode_b64_str(trimmed) {
        if decoded.contains("://") {
            return Ok(decoded);
        }
    }

    // 2. JSON config(s).
    if trimmed.starts_with('{') || trimmed.starts_with('[') {
        if let Some(links) = json_to_links(trimmed) {
            return Ok(links);
        }
    }

    // 3. Clash / Mihomo YAML.
    if looks_like_clash(trimmed) {
        if let Some(links) = yaml_to_links(trimmed) {
            return Ok(links);
        }
    }

    // 4. Plain text list of share links.
    if trimmed.contains("://") {
        return Ok(trimmed.to_string());
    }

    Err(Error::Parse("unrecognized subscription body".to_string()))
}

fn looks_like_clash(body: &str) -> bool {
    body.starts_with("proxies:") || body.contains("\nproxies:") || body.starts_with("---")
}

fn links_text(servers: &[Server]) -> String {
    servers
        .iter()
        .map(link::to_link)
        .collect::<Vec<_>>()
        .join("\n")
}

// ---------------------------------------------------------------------------
// JSON (Xray / sing-box)
// ---------------------------------------------------------------------------

fn json_to_links(body: &str) -> Option<String> {
    let json: Value = serde_json::from_str(body).ok()?;
    let servers = servers_from_json(&json);
    (!servers.is_empty()).then(|| links_text(&servers))
}

fn servers_from_json(value: &Value) -> Vec<Server> {
    match value {
        Value::Array(items) => items.iter().flat_map(servers_from_json).collect(),
        Value::Object(obj) => match obj.get("outbounds").and_then(Value::as_array) {
            Some(outbounds) => outbounds.iter().filter_map(outbound_to_server).collect(),
            None => outbound_to_server(value).into_iter().collect(),
        },
        _ => Vec::new(),
    }
}

fn outbound_to_server(value: &Value) -> Option<Server> {
    if value.get("protocol").and_then(Value::as_str).is_some() {
        return xray_outbound(value);
    }
    if value.get("type").and_then(Value::as_str).is_some() {
        return singbox_outbound(value);
    }
    None
}

fn xray_outbound(value: &Value) -> Option<Server> {
    let protocol = value.get("protocol")?.as_str()?;
    let settings = value.get("settings").cloned().unwrap_or(Value::Null);
    let name = opt_str(value, "tag").unwrap_or_default();
    let (address, port, credentials) = match protocol {
        "vless" => {
            let (address, port, user) = xray_vnext(&settings)?;
            (
                address,
                port,
                Credentials::Vless {
                    uuid: opt_str(&user, "id")?,
                    flow: opt_str(&user, "flow"),
                    encryption: opt_str(&user, "encryption"),
                },
            )
        }
        "vmess" => {
            let (address, port, user) = xray_vnext(&settings)?;
            (
                address,
                port,
                Credentials::Vmess {
                    uuid: opt_str(&user, "id")?,
                    alter_id: opt_u64(&user, "alterId").unwrap_or(0) as u32,
                    cipher: opt_str(&user, "security"),
                },
            )
        }
        "trojan" => {
            let server = first_server(&settings)?;
            (
                opt_str(server, "address")?,
                opt_u64(server, "port")? as u16,
                Credentials::Trojan {
                    password: opt_str(server, "password")?,
                },
            )
        }
        "shadowsocks" => {
            let server = first_server(&settings)?;
            (
                opt_str(server, "address")?,
                opt_u64(server, "port")? as u16,
                Credentials::Shadowsocks {
                    method: opt_str(server, "method")?,
                    password: opt_str(server, "password")?,
                    plugin: None,
                    plugin_opts: None,
                },
            )
        }
        _ => return None,
    };
    let (transport, security) = xray_stream(value.get("streamSettings"));
    Some(make_server(
        name,
        address,
        port,
        credentials,
        transport,
        security,
    ))
}

fn xray_vnext(settings: &Value) -> Option<(String, u16, Value)> {
    let entry = settings.get("vnext")?.as_array()?.first()?;
    let address = opt_str(entry, "address")?;
    let port = opt_u64(entry, "port")? as u16;
    let user = entry.get("users")?.as_array()?.first()?.clone();
    Some((address, port, user))
}

fn first_server(settings: &Value) -> Option<&Value> {
    settings.get("servers")?.as_array()?.first()
}

fn xray_stream(stream: Option<&Value>) -> (Transport, Security) {
    let Some(stream) = stream else {
        return (Transport::default(), Security::None);
    };
    let network = network_from_xray(
        stream
            .get("network")
            .and_then(Value::as_str)
            .unwrap_or("tcp"),
    );
    let mut transport = Transport {
        network,
        ..Transport::default()
    };
    match network {
        Network::Ws => {
            if let Some(ws) = stream.get("wsSettings") {
                transport.path = opt_str(ws, "path");
                transport.host = ws.get("headers").and_then(|h| opt_str(h, "Host"));
            }
        }
        Network::Grpc => {
            if let Some(grpc) = stream.get("grpcSettings") {
                transport.service_name = opt_str(grpc, "serviceName");
                transport.mode = opt_str(grpc, "mode");
            }
        }
        Network::HttpUpgrade => {
            if let Some(hu) = stream.get("httpupgradeSettings") {
                transport.path = opt_str(hu, "path");
                transport.host = opt_str(hu, "host");
            }
        }
        Network::Xhttp => {
            if let Some(xhttp) = stream.get("xhttpSettings") {
                transport.path = opt_str(xhttp, "path");
                transport.host = opt_str(xhttp, "host");
                transport.mode = opt_str(xhttp, "mode");
            }
        }
        Network::H2 => {
            if let Some(http) = stream.get("httpSettings") {
                transport.path = opt_str(http, "path");
                transport.host = first_str(http.get("host"));
            }
        }
        Network::Kcp => {
            if let Some(kcp) = stream.get("kcpSettings") {
                transport.header_type = kcp.get("header").and_then(|h| opt_str(h, "type"));
                transport.seed = opt_str(kcp, "seed");
            }
        }
        Network::Quic => {
            if let Some(quic) = stream.get("quicSettings") {
                transport.header_type = quic.get("header").and_then(|h| opt_str(h, "type"));
                transport.seed = opt_str(quic, "key");
            }
        }
        Network::Tcp => {
            if let Some(tcp) = stream.get("tcpSettings") {
                transport.header_type = tcp.get("header").and_then(|h| opt_str(h, "type"));
            }
        }
    }
    let security = match stream
        .get("security")
        .and_then(Value::as_str)
        .unwrap_or("none")
    {
        "tls" => tls_from_xray(stream.get("tlsSettings")),
        "reality" => reality_from_xray(stream.get("realitySettings")),
        _ => Security::None,
    };
    (transport, security)
}

fn tls_from_xray(settings: Option<&Value>) -> Security {
    let settings = settings.unwrap_or(&Value::Null);
    Security::Tls {
        sni: opt_str(settings, "serverName"),
        alpn: alpn_list(settings.get("alpn")),
        fingerprint: opt_str(settings, "fingerprint"),
        allow_insecure: opt_bool(settings, "allowInsecure").unwrap_or(false),
    }
}

fn reality_from_xray(settings: Option<&Value>) -> Security {
    let settings = settings.unwrap_or(&Value::Null);
    match opt_str(settings, "publicKey") {
        Some(public_key) => Security::Reality {
            sni: opt_str(settings, "serverName"),
            public_key,
            short_id: opt_str(settings, "shortId"),
            spider_x: opt_str(settings, "spiderX"),
            fingerprint: opt_str(settings, "fingerprint"),
        },
        None => tls_from_xray(Some(settings)),
    }
}

fn singbox_outbound(value: &Value) -> Option<Server> {
    let ty = value.get("type")?.as_str()?;
    let address = opt_str(value, "server")?;
    let port = opt_u64(value, "server_port")? as u16;
    let name = opt_str(value, "tag").unwrap_or_default();
    let (transport, security) = singbox_transport_security(value, ty);
    let credentials = match ty {
        "vless" => Credentials::Vless {
            uuid: opt_str(value, "uuid")?,
            flow: opt_str(value, "flow"),
            encryption: None,
        },
        "vmess" => Credentials::Vmess {
            uuid: opt_str(value, "uuid")?,
            alter_id: opt_u64(value, "alter_id").unwrap_or(0) as u32,
            cipher: opt_str(value, "security"),
        },
        "trojan" => Credentials::Trojan {
            password: opt_str(value, "password")?,
        },
        "shadowsocks" => Credentials::Shadowsocks {
            method: opt_str(value, "method")?,
            password: opt_str(value, "password")?,
            plugin: None,
            plugin_opts: None,
        },
        "hysteria2" => Credentials::Hysteria2 {
            password: opt_str(value, "password")?,
            obfs: value
                .get("obfs")
                .and_then(|o| opt_str(o, "type"))
                .or_else(|| opt_str(value, "obfs")),
            obfs_password: value.get("obfs").and_then(|o| opt_str(o, "password")),
            ports: opt_str(value, "ports"),
            up_mbps: opt_u64(value, "up_mbps").map(|n| n as u32),
            down_mbps: opt_u64(value, "down_mbps").map(|n| n as u32),
        },
        "tuic" => Credentials::Tuic {
            uuid: opt_str(value, "uuid")?,
            password: opt_str(value, "password")?,
            congestion_control: opt_str(value, "congestion_control"),
            udp_relay_mode: opt_str(value, "udp_relay_mode"),
        },
        _ => return None,
    };
    Some(make_server(
        name,
        address,
        port,
        credentials,
        transport,
        security,
    ))
}

fn singbox_transport_security(value: &Value, ty: &str) -> (Transport, Security) {
    let mut transport = Transport::default();
    if let Some(tr) = value.get("transport") {
        let kind = tr.get("type").and_then(Value::as_str).unwrap_or("tcp");
        transport.network = network_from_singbox(kind);
        match transport.network {
            Network::Ws | Network::HttpUpgrade => {
                transport.path = opt_str(tr, "path");
                transport.host = tr.get("headers").and_then(|h| opt_str(h, "Host"));
            }
            Network::Grpc => transport.service_name = opt_str(tr, "service_name"),
            Network::H2 => {
                transport.path = opt_str(tr, "path");
                transport.host = first_str(tr.get("host")).or_else(|| opt_str(tr, "host"));
            }
            _ => {}
        }
    }

    let security = if let Some(tls) = value.get("tls") {
        let enabled = opt_bool(tls, "enabled").unwrap_or(false);
        let reality = tls.get("reality");
        let reality_enabled = reality
            .map(|r| opt_bool(r, "enabled").unwrap_or(true))
            .unwrap_or(false);
        if reality_enabled {
            match reality.and_then(|r| opt_str(r, "public_key")) {
                Some(public_key) => Security::Reality {
                    sni: opt_str(tls, "server_name"),
                    public_key,
                    short_id: reality.and_then(|r| opt_str(r, "short_id")),
                    spider_x: None,
                    fingerprint: tls.get("utls").and_then(|u| opt_str(u, "fingerprint")),
                },
                None => singbox_tls(tls),
            }
        } else if enabled || matches!(ty, "hysteria2" | "tuic") {
            singbox_tls(tls)
        } else {
            Security::None
        }
    } else if matches!(ty, "hysteria2" | "tuic") {
        tls_default()
    } else {
        Security::None
    };

    (transport, security)
}

fn singbox_tls(tls: &Value) -> Security {
    Security::Tls {
        sni: opt_str(tls, "server_name"),
        alpn: alpn_list(tls.get("alpn")),
        fingerprint: tls.get("utls").and_then(|u| opt_str(u, "fingerprint")),
        allow_insecure: opt_bool(tls, "insecure").unwrap_or(false),
    }
}

// ---------------------------------------------------------------------------
// Clash / Mihomo YAML
// ---------------------------------------------------------------------------

fn yaml_to_links(body: &str) -> Option<String> {
    let yaml: serde_yaml::Value = serde_yaml::from_str(body).ok()?;
    let json = serde_json::to_value(yaml).ok()?;
    let proxies = json.get("proxies")?.as_array()?;
    let servers: Vec<Server> = proxies.iter().filter_map(clash_proxy).collect();
    (!servers.is_empty()).then(|| links_text(&servers))
}

fn clash_proxy(value: &Value) -> Option<Server> {
    let ty = opt_str(value, "type")?;
    let address = opt_str(value, "server")?;
    let port = opt_u64(value, "port")? as u16;
    let name = opt_str(value, "name").unwrap_or_default();
    let (transport, security) = clash_transport_security(value, &ty);
    let credentials = match ty.as_str() {
        "vless" => Credentials::Vless {
            uuid: opt_str(value, "uuid")?,
            flow: opt_str(value, "flow"),
            encryption: None,
        },
        "vmess" => Credentials::Vmess {
            uuid: opt_str(value, "uuid")?,
            alter_id: opt_u64(value, "alterId").unwrap_or(0) as u32,
            cipher: opt_str(value, "cipher"),
        },
        "trojan" => Credentials::Trojan {
            password: opt_str(value, "password")?,
        },
        "ss" => Credentials::Shadowsocks {
            method: opt_str(value, "cipher")?,
            password: opt_str(value, "password")?,
            plugin: opt_str(value, "plugin"),
            plugin_opts: None,
        },
        "hysteria2" => Credentials::Hysteria2 {
            password: opt_str(value, "password")?,
            obfs: opt_str(value, "obfs"),
            obfs_password: opt_str(value, "obfs-password"),
            ports: opt_str(value, "ports"),
            up_mbps: opt_u64(value, "up").map(|n| n as u32),
            down_mbps: opt_u64(value, "down").map(|n| n as u32),
        },
        "tuic" => Credentials::Tuic {
            uuid: opt_str(value, "uuid")?,
            password: opt_str(value, "password")?,
            congestion_control: opt_str(value, "congestion-controller"),
            udp_relay_mode: opt_str(value, "udp-relay-mode"),
        },
        _ => return None,
    };
    Some(make_server(
        name,
        address,
        port,
        credentials,
        transport,
        security,
    ))
}

fn clash_transport_security(value: &Value, ty: &str) -> (Transport, Security) {
    let mut transport = Transport::default();
    if let Some(network) = opt_str(value, "network") {
        transport.network = network_from_clash(&network);
        match transport.network {
            Network::Ws => {
                if let Some(ws) = value.get("ws-opts") {
                    transport.path = opt_str(ws, "path");
                    transport.host = ws.get("headers").and_then(|h| opt_str(h, "Host"));
                }
            }
            Network::Grpc => {
                transport.service_name = value
                    .get("grpc-opts")
                    .and_then(|g| opt_str(g, "grpc-service-name"));
            }
            Network::H2 => {
                if let Some(h2) = value.get("h2-opts") {
                    transport.path = opt_str(h2, "path");
                    transport.host = first_str(h2.get("host"));
                }
            }
            _ => {}
        }
    }

    let reality = value.get("reality-opts");
    let security = if reality.is_some() {
        match reality.and_then(|r| opt_str(r, "public-key")) {
            Some(public_key) => Security::Reality {
                sni: opt_str(value, "servername"),
                public_key,
                short_id: reality.and_then(|r| opt_str(r, "short-id")),
                spider_x: None,
                fingerprint: opt_str(value, "client-fingerprint"),
            },
            None => Security::None,
        }
    } else if opt_bool(value, "tls").unwrap_or(false) {
        Security::Tls {
            sni: opt_str(value, "servername").or_else(|| opt_str(value, "sni")),
            alpn: alpn_list(value.get("alpn")),
            fingerprint: opt_str(value, "client-fingerprint"),
            allow_insecure: opt_bool(value, "skip-cert-verify").unwrap_or(false),
        }
    } else if matches!(ty, "hysteria2" | "tuic") {
        Security::Tls {
            sni: opt_str(value, "sni"),
            alpn: alpn_list(value.get("alpn")),
            fingerprint: None,
            allow_insecure: opt_bool(value, "skip-cert-verify").unwrap_or(false),
        }
    } else {
        Security::None
    };

    (transport, security)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make_server(
    name: String,
    address: String,
    port: u16,
    credentials: Credentials,
    transport: Transport,
    security: Security,
) -> Server {
    Server {
        id: String::new(),
        name,
        country: None,
        address,
        port,
        credentials,
        transport,
        security,
        raw_link: None,
        traffic: None,
        local_traffic: TrafficInfo::default(),
        last_ping_ms: None,
    }
}

fn tls_default() -> Security {
    Security::Tls {
        sni: None,
        alpn: Vec::new(),
        fingerprint: None,
        allow_insecure: false,
    }
}

fn opt_str(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
        .filter(|s| !s.is_empty())
}

fn opt_u64(value: &Value, key: &str) -> Option<u64> {
    value.get(key).and_then(Value::as_u64)
}

fn opt_bool(value: &Value, key: &str) -> Option<bool> {
    value.get(key).and_then(Value::as_bool)
}

fn first_str(value: Option<&Value>) -> Option<String> {
    match value {
        Some(Value::Array(items)) => items.first().and_then(Value::as_str).map(str::to_string),
        Some(Value::String(s)) => Some(s.clone()),
        _ => None,
    }
}

fn alpn_list(value: Option<&Value>) -> Vec<String> {
    match value {
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect(),
        Some(Value::String(s)) => s
            .split(',')
            .map(|x| x.trim().to_string())
            .filter(|x| !x.is_empty())
            .collect(),
        _ => Vec::new(),
    }
}

fn network_from_xray(value: &str) -> Network {
    match value {
        "ws" => Network::Ws,
        "grpc" => Network::Grpc,
        "httpupgrade" => Network::HttpUpgrade,
        "xhttp" | "splithttp" => Network::Xhttp,
        "h2" => Network::H2,
        "kcp" => Network::Kcp,
        "quic" => Network::Quic,
        _ => Network::Tcp,
    }
}

fn network_from_singbox(value: &str) -> Network {
    match value {
        "ws" => Network::Ws,
        "grpc" => Network::Grpc,
        "http" => Network::H2,
        "httpupgrade" => Network::HttpUpgrade,
        "quic" => Network::Quic,
        _ => Network::Tcp,
    }
}

fn network_from_clash(value: &str) -> Network {
    match value {
        "ws" => Network::Ws,
        "grpc" => Network::Grpc,
        "h2" | "http" => Network::H2,
        _ => Network::Tcp,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_plain_links() {
        let text = "vless://uuid@h:443#A\ntrojan://pw@h:443#B\n";
        assert_eq!(decode_body(text).unwrap(), text.trim());
    }

    #[test]
    fn decodes_base64_std_and_urlsafe_without_padding() {
        use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
        use base64::Engine;
        let plain = "vless://uuid@h:443#A\nvmess://eyJ2IjoiMiJ9\n";
        let std = STANDARD.encode(plain);
        assert!(decode_body(&std).unwrap().contains("vless://uuid@h:443#A"));
        let urlsafe = URL_SAFE_NO_PAD.encode(plain);
        assert!(decode_body(&urlsafe)
            .unwrap()
            .contains("vless://uuid@h:443#A"));
    }

    #[test]
    fn rejects_garbage() {
        assert!(decode_body("just some text without links").is_err());
    }

    #[test]
    fn singbox_json_sample() {
        let json = r#"{
            "outbounds": [
                {"type":"vless","tag":"de","server":"de1.example.com","server_port":443,"uuid":"b831381d-6324-4d53-ad4f-8cda48b30811","flow":"xtls-rprx-vision","tls":{"enabled":true,"server_name":"sni.example.com","utls":{"enabled":true,"fingerprint":"chrome"}},"transport":{"type":"ws","path":"/ws","headers":{"Host":"host.example.com"}}},
                {"type":"hysteria2","tag":"hy","server":"hy.example.com","server_port":8443,"password":"secret","obfs":{"type":"salamander","password":"obfs-pw"}},
                {"type":"direct","tag":"direct"}
            ]
        }"#;
        let value: Value = serde_json::from_str(json).unwrap();
        let servers = servers_from_json(&value);
        assert_eq!(servers.len(), 2);

        let vless = &servers[0];
        assert_eq!(vless.name, "de");
        assert_eq!(vless.address, "de1.example.com");
        assert_eq!(vless.port, 443);
        assert_eq!(vless.transport.network, Network::Ws);
        assert_eq!(vless.transport.path.as_deref(), Some("/ws"));
        assert_eq!(vless.transport.host.as_deref(), Some("host.example.com"));
        match &vless.security {
            Security::Tls {
                sni, fingerprint, ..
            } => {
                assert_eq!(sni.as_deref(), Some("sni.example.com"));
                assert_eq!(fingerprint.as_deref(), Some("chrome"));
            }
            other => panic!("expected tls, got {other:?}"),
        }

        let hy = &servers[1];
        match &hy.credentials {
            Credentials::Hysteria2 {
                password,
                obfs,
                obfs_password,
                ..
            } => {
                assert_eq!(password, "secret");
                assert_eq!(obfs.as_deref(), Some("salamander"));
                assert_eq!(obfs_password.as_deref(), Some("obfs-pw"));
            }
            other => panic!("unexpected credentials {other:?}"),
        }
    }

    #[test]
    fn xray_json_sample() {
        let json = r#"{
            "outbounds": [
                {"protocol":"vless","tag":"x","settings":{"vnext":[{"address":"h","port":443,"users":[{"id":"u","flow":"xtls-rprx-vision"}]}]},"streamSettings":{"network":"ws","security":"tls","tlsSettings":{"serverName":"sni"},"wsSettings":{"path":"/p","headers":{"Host":"hhost"}}}},
                {"protocol":"shadowsocks","tag":"ss","settings":{"servers":[{"address":"s","port":8388,"method":"aes-256-gcm","password":"pw"}]}}
            ]
        }"#;
        let value: Value = serde_json::from_str(json).unwrap();
        let servers = servers_from_json(&value);
        assert_eq!(servers.len(), 2);
        assert_eq!(servers[0].transport.network, Network::Ws);
        assert_eq!(servers[0].transport.host.as_deref(), Some("hhost"));
        match &servers[1].credentials {
            Credentials::Shadowsocks {
                method, password, ..
            } => {
                assert_eq!(method, "aes-256-gcm");
                assert_eq!(password, "pw");
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn clash_yaml_sample() {
        let yaml = r#"
proxies:
  - name: "DE vless"
    type: vless
    server: de.example.com
    port: 443
    uuid: b831381d-6324-4d53-ad4f-8cda48b30811
    network: ws
    tls: true
    servername: sni.example.com
    ws-opts:
      path: /ws
      headers:
        Host: host.example.com
  - name: "SS"
    type: ss
    server: ss.example.com
    port: 8388
    cipher: aes-256-gcm
    password: pw
  - name: "HY2"
    type: hysteria2
    server: hy.example.com
    port: 8443
    password: secret
    sni: hy.example.com
"#;
        let value: Value = serde_yaml::from_str(yaml).unwrap();
        let value = serde_json::to_value(value).unwrap();
        let proxies = value.get("proxies").unwrap().as_array().unwrap();
        let servers: Vec<Server> = proxies.iter().filter_map(clash_proxy).collect();
        assert_eq!(servers.len(), 3);

        let vless = &servers[0];
        assert_eq!(vless.name, "DE vless");
        assert_eq!(vless.address, "de.example.com");
        assert_eq!(vless.transport.network, Network::Ws);
        assert_eq!(vless.transport.path.as_deref(), Some("/ws"));
        assert_eq!(vless.transport.host.as_deref(), Some("host.example.com"));
        assert!(matches!(vless.security, Security::Tls { .. }));

        assert!(matches!(
            servers[1].credentials,
            Credentials::Shadowsocks { .. }
        ));
        assert!(matches!(
            servers[2].credentials,
            Credentials::Hysteria2 { .. }
        ));
    }

    #[test]
    #[ignore = "needs T01 link parser"]
    fn decodes_json_to_links() {
        let json = r#"{"outbounds":[{"type":"vless","server":"h","server_port":443,"uuid":"u"}]}"#;
        let text = decode_body(json).unwrap();
        assert!(text.contains("vless://"));
    }
}

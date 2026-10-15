//! `vless://` and `trojan://` share links (they share the query parameter grammar).

use super::{encode_userinfo, finish_link, make_server, parse_parts, parse_userinfo, Parts, Query};
use crate::error::Error;
use crate::model::{Credentials, Network, Security, Server, Transport};
use crate::Result;

/// Parse a `vless://` (`trojan = false`) or `trojan://` (`trojan = true`) link body.
pub(super) fn parse(rest: &str, raw: &str, scope: &str, trojan: bool) -> Result<Server> {
    let parts = parse_parts(rest)?;
    let port = parts.port()?;
    let userinfo = parts
        .userinfo
        .clone()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| Error::Parse("missing userinfo".into()))?;

    let transport = parse_transport(&parts)?;
    let security = if trojan {
        match parts.get("security") {
            None | Some("") => default_tls(),
            _ => parse_security_pub(&parts)?,
        }
    } else {
        parse_security_pub(&parts)?
    };
    let traffic = parts.get("userinfo").and_then(parse_userinfo);

    let credentials = if trojan {
        Credentials::Trojan { password: userinfo }
    } else {
        Credentials::Vless {
            uuid: userinfo,
            flow: parts
                .get("flow")
                .filter(|s| !s.is_empty())
                .map(str::to_string),
            encryption: parts
                .get("encryption")
                .filter(|s| !s.is_empty() && !s.eq_ignore_ascii_case("none"))
                .map(str::to_string),
        }
    };

    let name = parts.remark(port);
    Ok(make_server(
        scope,
        raw,
        parts.host,
        port,
        credentials,
        transport,
        security,
        name,
        traffic,
    ))
}

fn default_tls() -> Security {
    Security::Tls {
        sni: None,
        alpn: Vec::new(),
        fingerprint: None,
        allow_insecure: false,
    }
}

/// Map the transport parameters (`type`, `host`, `path`, `serviceName`, ...).
pub(super) fn parse_transport(parts: &Parts) -> Result<Transport> {
    let mut transport = Transport::default();
    if let Some(kind) = parts.get("type") {
        transport.network = match kind.to_ascii_lowercase().as_str() {
            "tcp" | "raw" | "" => Network::Tcp,
            "ws" => Network::Ws,
            "grpc" => Network::Grpc,
            "httpupgrade" => Network::HttpUpgrade,
            "xhttp" | "splithttp" => Network::Xhttp,
            "h2" | "http" => Network::H2,
            "kcp" => Network::Kcp,
            "quic" => Network::Quic,
            _ => Network::Tcp,
        };
    }
    transport.path = parts
        .get("path")
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    transport.host = parts
        .get("host")
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    transport.service_name = parts
        .get("serviceName")
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    transport.mode = parts
        .get("mode")
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    transport.header_type = parts
        .get("headerType")
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    transport.seed = parts
        .get("seed")
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    if let Some(extra) = parts.get("extra").filter(|s| !s.is_empty()) {
        transport.extra = serde_json::from_str(extra).ok();
    }
    Ok(transport)
}

/// Map `security` / `sni` / `alpn` / `fp` / `pbk` / `sid` / `spx` / `allowInsecure`.
pub(super) fn parse_security_pub(parts: &Parts) -> Result<Security> {
    let kind = parts.get("security").unwrap_or("").to_ascii_lowercase();
    match kind.as_str() {
        "" | "none" => Ok(Security::None),
        "reality" => {
            let public_key = parts
                .get("pbk")
                .filter(|s| !s.is_empty())
                .ok_or_else(|| Error::Parse("reality link without pbk".into()))?
                .to_string();
            Ok(Security::Reality {
                sni: parts
                    .get("sni")
                    .filter(|s| !s.is_empty())
                    .map(str::to_string),
                public_key,
                short_id: parts
                    .get("sid")
                    .filter(|s| !s.is_empty())
                    .map(str::to_string),
                spider_x: parts
                    .get("spx")
                    .filter(|s| !s.is_empty())
                    .map(str::to_string),
                fingerprint: parts
                    .get("fp")
                    .filter(|s| !s.is_empty())
                    .map(str::to_string),
            })
        }
        "tls" => Ok(Security::Tls {
            sni: parts
                .get("sni")
                .filter(|s| !s.is_empty())
                .map(str::to_string),
            alpn: split_alpn(parts.get("alpn").unwrap_or("")),
            fingerprint: parts
                .get("fp")
                .filter(|s| !s.is_empty())
                .map(str::to_string),
            allow_insecure: truthy(parts.get("allowInsecure").unwrap_or("")),
        }),
        other => Err(Error::Parse(format!("unknown security {other}"))),
    }
}

pub(super) fn split_alpn(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

pub(super) fn truthy(value: &str) -> bool {
    matches!(value.to_ascii_lowercase().as_str(), "1" | "true" | "yes")
}

/// Export a vless / trojan link.
pub(super) fn to_link(server: &Server, trojan: bool) -> String {
    let (scheme, userinfo) = if trojan {
        let password = match &server.credentials {
            Credentials::Trojan { password } => password.clone(),
            _ => String::new(),
        };
        ("trojan", password)
    } else {
        let uuid = match &server.credentials {
            Credentials::Vless { uuid, .. } => uuid.clone(),
            _ => String::new(),
        };
        ("vless", uuid)
    };

    let mut query = Query::new();
    if server.transport.network != Network::Tcp {
        query.add("type", server.transport.network.as_str());
    } else {
        query.add("type", "tcp");
    }
    if let Credentials::Vless {
        flow, encryption, ..
    } = &server.credentials
    {
        if let Some(enc) = encryption.as_deref().filter(|s| !s.is_empty()) {
            query.add("encryption", enc);
        } else {
            query.add("encryption", "none");
        }
        if let Some(flow) = flow.as_deref().filter(|s| !s.is_empty()) {
            query.add("flow", flow);
        }
    }
    add_security(&mut query, &server.security);

    let t = &server.transport;
    query.add("path", t.path.as_deref().unwrap_or(""));
    query.add("host", t.host.as_deref().unwrap_or(""));
    query.add("serviceName", t.service_name.as_deref().unwrap_or(""));
    query.add("mode", t.mode.as_deref().unwrap_or(""));
    query.add("headerType", t.header_type.as_deref().unwrap_or(""));
    query.add("seed", t.seed.as_deref().unwrap_or(""));
    if let Some(extra) = &t.extra {
        query.add_owned("extra", extra.to_string());
    }
    if let Some(traffic) = &server.traffic {
        query.add("userinfo", &format_userinfo(traffic));
    }

    let base = format!(
        "{scheme}://{}@{}",
        encode_userinfo(&userinfo),
        super::format_host_port(&server.address, server.port)
    );
    finish_link(base, &query, &server.name)
}

pub(super) fn add_security(query: &mut Query, security: &Security) {
    match security {
        Security::None => query.add("security", "none"),
        Security::Tls {
            sni,
            alpn,
            fingerprint,
            allow_insecure,
        } => {
            query.add("security", "tls");
            query.add("sni", sni.as_deref().unwrap_or(""));
            if !alpn.is_empty() {
                query.add("alpn", &alpn.join(","));
            }
            query.add("fp", fingerprint.as_deref().unwrap_or(""));
            query.add_flag("allowInsecure", *allow_insecure);
        }
        Security::Reality {
            sni,
            public_key,
            short_id,
            spider_x,
            fingerprint,
        } => {
            query.add("security", "reality");
            query.add("sni", sni.as_deref().unwrap_or(""));
            query.add("pbk", public_key);
            query.add("sid", short_id.as_deref().unwrap_or(""));
            query.add("spx", spider_x.as_deref().unwrap_or(""));
            query.add("fp", fingerprint.as_deref().unwrap_or(""));
        }
    }
}

pub(super) fn format_userinfo(traffic: &crate::model::TrafficInfo) -> String {
    let mut s = format!("upload={}; download={}", traffic.upload, traffic.download);
    if let Some(total) = traffic.total {
        s.push_str(&format!("; total={total}"));
    }
    if let Some(expire) = traffic.expire {
        s.push_str(&format!("; expire={expire}"));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::link::parse_link;

    #[test]
    fn vless_reality_vision() {
        let link = "vless://b831381d-6324-4d53-ad4f-8cda48b30811@de1.example.com:443?type=tcp&security=reality&sni=www.microsoft.com&fp=chrome&pbk=BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB&sid=abcd1234&spx=%2F&flow=xtls-rprx-vision#%F0%9F%87%A9%F0%9F%87%AA%20Germany%20%231";
        let s = parse_link(link, "local").unwrap();
        assert_eq!(s.address, "de1.example.com");
        assert_eq!(s.port, 443);
        assert_eq!(s.country.as_deref(), Some("DE"));
        match &s.credentials {
            Credentials::Vless {
                uuid,
                flow,
                encryption,
            } => {
                assert_eq!(uuid, "b831381d-6324-4d53-ad4f-8cda48b30811");
                assert_eq!(flow.as_deref(), Some("xtls-rprx-vision"));
                assert!(encryption.is_none());
            }
            _ => panic!(),
        }
        match &s.security {
            Security::Reality {
                public_key,
                short_id,
                spider_x,
                ..
            } => {
                assert_eq!(public_key, "BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB");
                assert_eq!(short_id.as_deref(), Some("abcd1234"));
                assert_eq!(spider_x.as_deref(), Some("/"));
            }
            _ => panic!("expected reality"),
        }
    }

    #[test]
    fn vless_ws_tls_alpn() {
        let link = "vless://uuid-1@example.com:8443?type=ws&security=tls&sni=cdn.example.com&host=cdn.example.com&path=%2Fws&fp=firefox&alpn=h2,http%2F1.1&encryption=none#Node%201";
        let s = parse_link(link, "local").unwrap();
        assert_eq!(s.transport.network, Network::Ws);
        assert_eq!(s.transport.path.as_deref(), Some("/ws"));
        assert_eq!(s.transport.host.as_deref(), Some("cdn.example.com"));
        match &s.security {
            Security::Tls {
                sni,
                alpn,
                fingerprint,
                allow_insecure,
            } => {
                assert_eq!(sni.as_deref(), Some("cdn.example.com"));
                assert_eq!(alpn, &["h2", "http/1.1"]);
                assert_eq!(fingerprint.as_deref(), Some("firefox"));
                assert!(!allow_insecure);
            }
            _ => panic!(),
        }
    }

    #[test]
    fn vless_grpc_and_xhttp() {
        let grpc = "vless://u@example.com:443?type=grpc&security=tls&serviceName=mysvc&mode=gun&sni=x.com#g";
        let s = parse_link(grpc, "local").unwrap();
        assert_eq!(s.transport.network, Network::Grpc);
        assert_eq!(s.transport.service_name.as_deref(), Some("mysvc"));
        assert_eq!(s.transport.mode.as_deref(), Some("gun"));

        let xhttp = "vless://u@example.com:443?type=xhttp&security=tls&path=%2Fx&host=h.com&mode=packet-up&extra=%7B%22xPaddingBytes%22%3A%22100-1000%22%7D#x";
        let s = parse_link(xhttp, "local").unwrap();
        assert_eq!(s.transport.network, Network::Xhttp);
        assert_eq!(s.transport.mode.as_deref(), Some("packet-up"));
        assert_eq!(
            s.transport.extra.as_ref().unwrap()["xPaddingBytes"],
            "100-1000"
        );
    }

    #[test]
    fn vless_raw_maps_to_tcp_and_ipv6() {
        let s = parse_link(
            "vless://u@[2001:db8::1]:443?type=raw&security=none#v6",
            "local",
        )
        .unwrap();
        assert_eq!(s.address, "2001:db8::1");
        assert_eq!(s.transport.network, Network::Tcp);
        assert_eq!(s.security, Security::None);
    }

    #[test]
    fn trojan_defaults_to_tls() {
        let s = parse_link(
            "trojan://pass%40word@example.com:443?sni=example.com#T",
            "local",
        )
        .unwrap();
        assert_eq!(s.address, "example.com");
        assert_eq!(s.transport.network, Network::Tcp);
        match &s.credentials {
            Credentials::Trojan { password } => assert_eq!(password, "pass@word"),
            _ => panic!(),
        }
        assert!(matches!(s.security, Security::Tls { .. }));
    }

    #[test]
    fn trojan_ws() {
        let link = "trojan://secret@example.com:8443?type=ws&security=tls&path=%2Ftj&host=cdn.com&sni=cdn.com#Trojan%20WS";
        let s = parse_link(link, "local").unwrap();
        assert_eq!(s.transport.network, Network::Ws);
        assert_eq!(s.name, "Trojan WS");
    }
}

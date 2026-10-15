//! `tuic://` links (QUIC + TLS).

use super::vless::{split_alpn, truthy};
use super::{encode_userinfo, finish_link, make_server, parse_parts, parse_userinfo, Query};
use crate::error::Error;
use crate::model::{Credentials, Security, Server, Transport};
use crate::Result;

pub(super) fn parse(rest: &str, raw: &str, scope: &str) -> Result<Server> {
    let parts = parse_parts(rest)?;
    let userinfo = parts
        .userinfo
        .clone()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| Error::Parse("tuic: missing userinfo".into()))?;
    let (uuid, password) = userinfo
        .split_once(':')
        .ok_or_else(|| Error::Parse("tuic: expected uuid:password".into()))?;
    let port = parts.port()?;

    let traffic = parts.get("userinfo").and_then(parse_userinfo);
    let name = parts.remark(port);
    let congestion_control = parts
        .get("congestion_control")
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let udp_relay_mode = parts
        .get("udp_relay_mode")
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let sni = parts
        .get("sni")
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let alpn = split_alpn(parts.get("alpn").unwrap_or(""));
    let allow_insecure = truthy(parts.get("allow_insecure").unwrap_or(""))
        || truthy(parts.get("insecure").unwrap_or(""));

    Ok(make_server(
        scope,
        raw,
        parts.host,
        port,
        Credentials::Tuic {
            uuid: uuid.to_string(),
            password: password.to_string(),
            congestion_control,
            udp_relay_mode,
        },
        Transport::default(),
        Security::Tls {
            sni,
            alpn,
            fingerprint: None,
            allow_insecure,
        },
        name,
        traffic,
    ))
}

pub(super) fn to_link(server: &Server) -> String {
    let (uuid, password, cc, udp) = match &server.credentials {
        Credentials::Tuic {
            uuid,
            password,
            congestion_control,
            udp_relay_mode,
        } => (
            uuid.clone(),
            password.clone(),
            congestion_control.clone(),
            udp_relay_mode.clone(),
        ),
        _ => (String::new(), String::new(), None, None),
    };
    let (sni, alpn, allow_insecure) = match &server.security {
        Security::Tls {
            sni,
            alpn,
            allow_insecure,
            ..
        } => (sni.clone(), alpn.clone(), *allow_insecure),
        _ => (None, Vec::new(), false),
    };

    let mut query = Query::new();
    query.add("congestion_control", cc.as_deref().unwrap_or(""));
    query.add("udp_relay_mode", udp.as_deref().unwrap_or(""));
    if !alpn.is_empty() {
        query.add("alpn", &alpn.join(","));
    }
    query.add("sni", sni.as_deref().unwrap_or(""));
    query.add_flag("allow_insecure", allow_insecure);
    if let Some(traffic) = &server.traffic {
        query.add("userinfo", &super::vless::format_userinfo(traffic));
    }

    let base = format!(
        "tuic://{}:{}@{}",
        encode_userinfo(&uuid),
        encode_userinfo(&password),
        super::format_host_port(&server.address, server.port)
    );
    finish_link(base, &query, &server.name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::link::parse_link;

    #[test]
    fn tuic_full() {
        let link = "tuic://a1b2-uuid:p%40ss@tuic.example.com:443?congestion_control=bbr&udp_relay_mode=native&alpn=h3&sni=tuic.example.com&allow_insecure=1#TUIC";
        let s = parse_link(link, "local").unwrap();
        assert_eq!(s.address, "tuic.example.com");
        assert_eq!(s.port, 443);
        match &s.credentials {
            Credentials::Tuic {
                uuid,
                password,
                congestion_control,
                udp_relay_mode,
            } => {
                assert_eq!(uuid, "a1b2-uuid");
                assert_eq!(password, "p@ss");
                assert_eq!(congestion_control.as_deref(), Some("bbr"));
                assert_eq!(udp_relay_mode.as_deref(), Some("native"));
            }
            _ => panic!(),
        }
        match &s.security {
            Security::Tls {
                alpn,
                allow_insecure,
                ..
            } => {
                assert_eq!(alpn, &["h3"]);
                assert!(allow_insecure);
            }
            _ => panic!(),
        }
    }

    #[test]
    fn tuic_minimal() {
        let s = parse_link("tuic://u:p@1.2.3.4:8443#t", "local").unwrap();
        assert_eq!(s.port, 8443);
        assert_eq!(s.name, "t");
        assert!(matches!(s.security, Security::Tls { .. }));
    }

    #[test]
    fn tuic_roundtrip() {
        let link = "tuic://u:p%40w@tuic.example.com:443?congestion_control=bbr&udp_relay_mode=native&alpn=h3&sni=tuic.example.com#RT";
        let a = parse_link(link, "local").unwrap();
        let b = parse_link(&to_link(&a), "local").unwrap();
        assert_eq!(a.credentials, b.credentials);
        assert_eq!(a.security, b.security);
        assert_eq!(a.name, b.name);
    }
}

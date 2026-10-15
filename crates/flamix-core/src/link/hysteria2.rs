//! `hysteria2://` / `hy2://` links (TLS is always on).

use super::vless::{split_alpn, truthy};
use super::{encode_userinfo, finish_link, make_server, parse_parts, parse_userinfo, Query};
use crate::error::Error;
use crate::model::{Credentials, Security, Server, Transport};
use crate::Result;

pub(super) fn parse(rest: &str, raw: &str, scope: &str) -> Result<Server> {
    let parts = parse_parts(rest)?;
    let password = parts
        .userinfo
        .clone()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| Error::Parse("hysteria2: missing password".into()))?;
    let (port, ports) = parse_port_range(parts.port_raw.as_deref().unwrap_or(""))?;
    let ports = ports.or_else(|| {
        parts
            .get("mport")
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    });

    let sni = parts
        .get("sni")
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let alpn = split_alpn(parts.get("alpn").unwrap_or(""));
    let allow_insecure = truthy(parts.get("insecure").unwrap_or(""))
        || truthy(parts.get("allow_insecure").unwrap_or(""));

    let obfs = parts
        .get("obfs")
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let obfs_password = parts
        .get("obfs-password")
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let up_mbps = parts.get("up").and_then(|s| s.parse().ok());
    let down_mbps = parts.get("down").and_then(|s| s.parse().ok());
    let traffic = parts.get("userinfo").and_then(parse_userinfo);

    let name = parts.remark(port);
    Ok(make_server(
        scope,
        raw,
        parts.host,
        port,
        Credentials::Hysteria2 {
            password,
            obfs,
            obfs_password,
            ports,
            up_mbps,
            down_mbps,
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

/// Parse `443` or `443,20000-30000` into (port, optional range).
fn parse_port_range(s: &str) -> Result<(u16, Option<String>)> {
    let mut it = s.splitn(2, ',');
    let first = it.next().unwrap_or("");
    let port = super::parse_port(first)
        .ok_or_else(|| Error::Parse("hysteria2: missing or bad port".into()))?;
    let ports = it
        .next()
        .map(str::trim)
        .filter(|r| !r.is_empty())
        .map(str::to_string);
    Ok((port, ports))
}

pub(super) fn to_link(server: &Server) -> String {
    let (password, obfs, obfs_password, ports, up, down) = match &server.credentials {
        Credentials::Hysteria2 {
            password,
            obfs,
            obfs_password,
            ports,
            up_mbps,
            down_mbps,
        } => (
            password.clone(),
            obfs.clone(),
            obfs_password.clone(),
            ports.clone(),
            *up_mbps,
            *down_mbps,
        ),
        _ => (String::new(), None, None, None, None, None),
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
    query.add("sni", sni.as_deref().unwrap_or(""));
    query.add_flag("insecure", allow_insecure);
    query.add("obfs", obfs.as_deref().unwrap_or(""));
    query.add("obfs-password", obfs_password.as_deref().unwrap_or(""));
    query.add("mport", ports.as_deref().unwrap_or(""));
    if !alpn.is_empty() {
        query.add("alpn", &alpn.join(","));
    }
    if let Some(up) = up {
        query.add("up", &up.to_string());
    }
    if let Some(down) = down {
        query.add("down", &down.to_string());
    }
    if let Some(traffic) = &server.traffic {
        query.add("userinfo", &super::vless::format_userinfo(traffic));
    }

    let host_port = super::format_host_port(&server.address, server.port);
    let host_part = match &ports {
        Some(p) => format!("{host_port},{p}"),
        None => host_port,
    };
    let base = format!("hysteria2://{}@{host_part}", encode_userinfo(&password));
    finish_link(base, &query, &server.name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::link::parse_link;

    #[test]
    fn hy2_obfs_and_hop() {
        let link = "hysteria2://letmein@hy.example.com:443,20000-30000?sni=hy.example.com&insecure=1&obfs=salamander&obfs-password=obfspw&alpn=h3&mport=20000-30000#%F0%9F%87%B3%F0%9F%87%B1%20NL";
        let s = parse_link(link, "local").unwrap();
        assert_eq!(s.address, "hy.example.com");
        assert_eq!(s.port, 443);
        assert_eq!(s.country.as_deref(), Some("NL"));
        match &s.credentials {
            Credentials::Hysteria2 {
                password,
                obfs,
                obfs_password,
                ports,
                ..
            } => {
                assert_eq!(password, "letmein");
                assert_eq!(obfs.as_deref(), Some("salamander"));
                assert_eq!(obfs_password.as_deref(), Some("obfspw"));
                assert_eq!(ports.as_deref(), Some("20000-30000"));
            }
            _ => panic!(),
        }
        assert!(matches!(
            s.security,
            Security::Tls {
                allow_insecure: true,
                ..
            }
        ));
    }

    #[test]
    fn hy2_short_scheme_plain() {
        let s = parse_link("hy2://pw@1.2.3.4:8443?sni=x.com#plain", "local").unwrap();
        assert_eq!(s.port, 8443);
        assert_eq!(s.name, "plain");
    }

    #[test]
    fn hy2_no_name_uses_host_port() {
        let s = parse_link("hysteria2://pw@1.2.3.4:8443", "local").unwrap();
        assert_eq!(s.name, "1.2.3.4:8443");
        assert_eq!(s.country, None);
    }

    #[test]
    fn hy2_roundtrip() {
        let link = "hysteria2://pw@hy.example.com:443,20000-30000?sni=hy.example.com&insecure=1&obfs=salamander&obfs-password=x&alpn=h3&mport=20000-30000#RT";
        let a = parse_link(link, "local").unwrap();
        let b = parse_link(&to_link(&a), "local").unwrap();
        assert_eq!(a.credentials, b.credentials);
        assert_eq!(a.security, b.security);
        assert_eq!(a.port, b.port);
        assert_eq!(a.name, b.name);
    }
}

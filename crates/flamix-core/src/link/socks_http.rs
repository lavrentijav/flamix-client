//! `socks://` / `socks5://` and `http://` / `https://` proxy links.

use super::{
    decode_base64_any, encode_userinfo, finish_link, make_server, parse_parts, parse_userinfo,
    Query,
};
use crate::error::Error;
use crate::model::{Credentials, Security, Server, Transport};
use crate::Result;

pub(super) fn parse(scheme: &str, rest: &str, raw: &str, scope: &str) -> Result<Server> {
    let parts = parse_parts(rest)?;
    let is_http = scheme == "http" || scheme == "https";
    let default_port = if is_http { 8080 } else { 1080 };
    let port = match parts.port_raw.as_deref() {
        Some(p) if !p.is_empty() => {
            super::parse_port(p).ok_or_else(|| Error::Parse("bad port".into()))?
        }
        _ => default_port,
    };

    let (username, password) = decode_creds(parts.userinfo.as_deref().unwrap_or(""));
    let security = if scheme == "https" {
        Security::Tls {
            sni: None,
            alpn: Vec::new(),
            fingerprint: None,
            allow_insecure: false,
        }
    } else {
        Security::None
    };
    let credentials = if is_http {
        Credentials::Http { username, password }
    } else {
        Credentials::Socks { username, password }
    };
    let traffic = parts.get("userinfo").and_then(parse_userinfo);
    let name = parts.remark(port);
    Ok(make_server(
        scope,
        raw,
        parts.host,
        port,
        credentials,
        Transport::default(),
        security,
        name,
        traffic,
    ))
}

fn decode_creds(userinfo: &str) -> (Option<String>, Option<String>) {
    if userinfo.is_empty() {
        return (None, None);
    }
    if let Some((user, pass)) = userinfo.split_once(':') {
        return (Some(user.to_string()), Some(pass.to_string()));
    }
    if let Some(decoded) = decode_base64_any(userinfo) {
        if let Ok(text) = String::from_utf8(decoded) {
            return match text.split_once(':') {
                Some((user, pass)) => (Some(user.to_string()), Some(pass.to_string())),
                None => (Some(text), None),
            };
        }
    }
    (Some(userinfo.to_string()), None)
}

pub(super) fn to_link(server: &Server) -> String {
    let (username, password) = match &server.credentials {
        Credentials::Socks { username, password } | Credentials::Http { username, password } => {
            (username.clone(), password.clone())
        }
        _ => (None, None),
    };
    let scheme = match &server.credentials {
        Credentials::Http { .. } => {
            if matches!(server.security, Security::Tls { .. }) {
                "https"
            } else {
                "http"
            }
        }
        _ => "socks",
    };

    let userinfo = match (username, password) {
        (Some(user), Some(pass)) => {
            format!("{}:{}@", encode_userinfo(&user), encode_userinfo(&pass))
        }
        (Some(user), None) => format!("{}@", encode_userinfo(&user)),
        _ => String::new(),
    };
    let mut query = Query::new();
    if let Some(traffic) = &server.traffic {
        query.add("userinfo", &super::vless::format_userinfo(traffic));
    }
    let base = format!(
        "{scheme}://{userinfo}{}",
        super::format_host_port(&server.address, server.port)
    );
    finish_link(base, &query, &server.name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::link::parse_link;

    #[test]
    fn socks5_auth() {
        let s = parse_link("socks5://user:pass@127.0.0.1:1080#S", "local").unwrap();
        assert_eq!(s.address, "127.0.0.1");
        assert_eq!(s.port, 1080);
        match &s.credentials {
            Credentials::Socks { username, password } => {
                assert_eq!(username.as_deref(), Some("user"));
                assert_eq!(password.as_deref(), Some("pass"));
            }
            _ => panic!(),
        }
    }

    #[test]
    fn socks_base64_userinfo() {
        use base64::Engine;
        let ui = base64::engine::general_purpose::STANDARD.encode("bob:secret");
        let link = format!("socks://{ui}@proxy.example.com:1080#B64");
        let s = parse_link(&link, "local").unwrap();
        match &s.credentials {
            Credentials::Socks { username, password } => {
                assert_eq!(username.as_deref(), Some("bob"));
                assert_eq!(password.as_deref(), Some("secret"));
            }
            _ => panic!(),
        }
    }

    #[test]
    fn http_and_https() {
        let http = parse_link("http://u:p@proxy.example.com:8080#H", "local").unwrap();
        assert_eq!(http.security, Security::None);
        assert_eq!(http.port, 8080);

        let https = parse_link("https://proxy.example.com:8443#HS", "local").unwrap();
        assert!(matches!(https.security, Security::Tls { .. }));
        assert_eq!(https.port, 8443);
    }

    #[test]
    fn socks_roundtrip() {
        let a = parse_link("socks://user:p%40ss@127.0.0.1:1080#RT", "local").unwrap();
        let b = parse_link(&to_link(&a), "local").unwrap();
        assert_eq!(a.credentials, b.credentials);
        assert_eq!(a.address, b.address);
        assert_eq!(a.name, b.name);
    }

    #[test]
    fn http_roundtrip_https() {
        let a = parse_link("https://u:p@proxy.example.com:8443#RT", "local").unwrap();
        let b = parse_link(&to_link(&a), "local").unwrap();
        assert_eq!(a.credentials, b.credentials);
        assert_eq!(a.security, b.security);
        assert_eq!(a.name, b.name);
    }
}

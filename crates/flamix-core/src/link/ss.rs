//! `ss://` Shadowsocks links: SIP002 (`base64(method:password)@host:port`) and the
//! legacy `base64(method:password@host:port)` form, with SIP003 plugin support.

use super::{
    decode_base64_any, finish_link, make_server, parse_query, percent_decode, split_fragment,
    split_host_port, split_query, Query,
};
use crate::error::Error;
use crate::model::{Credentials, Security, Server, Transport};
use crate::Result;

pub(super) fn parse(rest: &str, raw: &str, scope: &str) -> Result<Server> {
    let (body, fragment) = split_fragment(rest);
    let (method, password, host, port, plugin) = if body.contains('@') {
        parse_sip002(body)?
    } else {
        parse_legacy(body)?
    };
    let port = port.ok_or_else(|| Error::Parse("ss: missing port".into()))?;
    let (plugin, plugin_opts) = plugin;
    let traffic = None;
    let name = super::remark(&fragment.map(percent_decode), &host, port);
    Ok(make_server(
        scope,
        raw,
        host,
        port,
        Credentials::Shadowsocks {
            method,
            password,
            plugin,
            plugin_opts,
        },
        Transport::default(),
        Security::None,
        name,
        traffic,
    ))
}

type SsParts = (
    String,
    String,
    String,
    Option<u16>,
    (Option<String>, Option<String>),
);

/// `ss://userinfo@host:port/?plugin=...#name` (SIP002).
fn parse_sip002(body: &str) -> Result<SsParts> {
    let (authority, query) = split_query(body);
    let authority = authority.strip_suffix('/').unwrap_or(authority);
    let (userinfo, hostport) = authority
        .rsplit_once('@')
        .ok_or_else(|| Error::Parse("ss: no userinfo".into()))?;
    let userinfo = percent_decode(userinfo);
    let (method, password) =
        decode_userinfo(&userinfo).ok_or_else(|| Error::Parse("ss: bad userinfo".into()))?;
    let (host, port) = split_host_port(hostport)?;
    let port = match port {
        Some(p) => Some(super::parse_port(&p).ok_or_else(|| Error::Parse("ss: bad port".into()))?),
        None => None,
    };
    let mut plugin = (None, None);
    for (k, v) in parse_query(query.unwrap_or("")) {
        if k.eq_ignore_ascii_case("plugin") && !v.is_empty() {
            plugin = split_plugin(&v);
        }
    }
    Ok((method, password, host, port, plugin))
}

/// Legacy `ss://base64(method:password@host:port)#name`.
fn parse_legacy(body: &str) -> Result<SsParts> {
    let decoded =
        decode_base64_any(body).ok_or_else(|| Error::Parse("ss: invalid base64".into()))?;
    let text =
        String::from_utf8(decoded).map_err(|_| Error::Parse("ss: payload is not UTF-8".into()))?;
    let (userinfo, hostport) = text
        .rsplit_once('@')
        .ok_or_else(|| Error::Parse("ss: legacy link without userinfo".into()))?;
    let (method, password) =
        decode_userinfo(userinfo).ok_or_else(|| Error::Parse("ss: bad userinfo".into()))?;
    let (host, port) = split_host_port(hostport)?;
    let port = match port {
        Some(p) => Some(super::parse_port(&p).ok_or_else(|| Error::Parse("ss: bad port".into()))?),
        None => None,
    };
    Ok((method, password, host, port, (None, None)))
}

/// Decode `method:password`, base64 or plain (percent-decoded already).
fn decode_userinfo(userinfo: &str) -> Option<(String, String)> {
    if let Some(decoded) = decode_base64_any(userinfo) {
        if let Ok(text) = String::from_utf8(decoded) {
            if let Some((m, p)) = text.split_once(':') {
                return Some((m.to_string(), p.to_string()));
            }
        }
    }
    if let Some((m, p)) = userinfo.split_once(':') {
        return Some((m.to_string(), p.to_string()));
    }
    None
}

fn split_plugin(value: &str) -> (Option<String>, Option<String>) {
    match value.split_once(';') {
        Some((name, opts)) => (Some(name.to_string()), Some(opts.to_string())),
        None => (Some(value.to_string()), None),
    }
}

/// Export as SIP002 with the userinfo base64-encoded.
pub(super) fn to_link(server: &Server) -> String {
    let (method, password, plugin, plugin_opts) = match &server.credentials {
        Credentials::Shadowsocks {
            method,
            password,
            plugin,
            plugin_opts,
        } => (
            method.clone(),
            password.clone(),
            plugin.clone(),
            plugin_opts.clone(),
        ),
        _ => (String::new(), String::new(), None, None),
    };
    use base64::Engine;
    let userinfo =
        base64::engine::general_purpose::STANDARD_NO_PAD.encode(format!("{method}:{password}"));
    let mut query = Query::new();
    if let Some(plugin) = plugin {
        let value = match plugin_opts {
            Some(opts) if !opts.is_empty() => format!("{plugin};{opts}"),
            _ => plugin,
        };
        query.add("plugin", &value);
    }
    let base = format!(
        "ss://{userinfo}@{}",
        super::format_host_port(&server.address, server.port)
    );
    finish_link(base, &query, &server.name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::link::parse_link;

    fn b64(s: &str) -> String {
        use base64::Engine;
        base64::engine::general_purpose::STANDARD.encode(s.as_bytes())
    }

    #[test]
    fn ss_sip002_base64_userinfo() {
        let userinfo = b64("aes-256-gcm:secret");
        let link = format!("ss://{userinfo}@ss.example.com:8388#Node");
        let s = parse_link(&link, "local").unwrap();
        assert_eq!(s.address, "ss.example.com");
        assert_eq!(s.port, 8388);
        match &s.credentials {
            Credentials::Shadowsocks {
                method, password, ..
            } => {
                assert_eq!(method, "aes-256-gcm");
                assert_eq!(password, "secret");
            }
            _ => panic!(),
        }
    }

    #[test]
    fn ss_sip002_plain_userinfo() {
        let link = "ss://aes-128-gcm:p%40ss@ss.example.com:8388#Plain";
        let s = parse_link(link, "local").unwrap();
        match &s.credentials {
            Credentials::Shadowsocks {
                method, password, ..
            } => {
                assert_eq!(method, "aes-128-gcm");
                assert_eq!(password, "p@ss");
            }
            _ => panic!(),
        }
    }

    #[test]
    fn ss_legacy() {
        let link = format!(
            "ss://{}#Legacy",
            b64("chacha20-ietf-poly1305:pw@1.2.3.4:8388")
        );
        let s = parse_link(&link, "local").unwrap();
        assert_eq!(s.address, "1.2.3.4");
        assert_eq!(s.port, 8388);
        assert_eq!(s.name, "Legacy");
    }

    #[test]
    fn ss_plugin() {
        let userinfo = b64("aes-256-gcm:secret");
        let link = format!(
            "ss://{userinfo}@ss.example.com:8388/?plugin=obfs-local%3Bobfs%3Dhttp%3Bobfs-host%3Dcloudflare.com#Plugin"
        );
        let s = parse_link(&link, "local").unwrap();
        match &s.credentials {
            Credentials::Shadowsocks {
                plugin,
                plugin_opts,
                ..
            } => {
                assert_eq!(plugin.as_deref(), Some("obfs-local"));
                assert_eq!(
                    plugin_opts.as_deref(),
                    Some("obfs=http;obfs-host=cloudflare.com")
                );
            }
            _ => panic!(),
        }
    }

    #[test]
    fn ss_roundtrip() {
        let link = "ss://YWVzLTI1Ni1nY206c2VjcmV0@ss.example.com:8388/?plugin=v2ray-plugin%3Bmode%3Dwebsocket#RT";
        let a = parse_link(link, "local").unwrap();
        let b = parse_link(&to_link(&a), "local").unwrap();
        assert_eq!(a.credentials, b.credentials);
        assert_eq!(a.address, b.address);
        assert_eq!(a.port, b.port);
        assert_eq!(a.name, b.name);
    }
}

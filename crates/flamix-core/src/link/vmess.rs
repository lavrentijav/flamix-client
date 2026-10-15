//! `vmess://` links: v2rayN base64 JSON form and the `uuid@host:port` URI form.

use super::vless::{format_userinfo, parse_transport, split_alpn, truthy};
use super::{decode_base64_any, make_server, parse_parts, parse_userinfo};
use crate::error::Error;
use crate::model::{Credentials, Network, Security, Server, Transport};
use crate::Result;
use serde_json::Value;

pub(super) fn parse(rest: &str, raw: &str, scope: &str) -> Result<Server> {
    let (body, fragment) = super::split_fragment(rest);
    if body.contains('@') {
        parse_uri(body, fragment, raw, scope)
    } else {
        parse_json(rest, raw, scope)
    }
}

fn parse_uri(body: &str, fragment: Option<&str>, raw: &str, scope: &str) -> Result<Server> {
    let parts = parse_parts(body)?;
    let port = parts.port()?;
    let uuid = parts
        .userinfo
        .clone()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| Error::Parse("vmess URI without uuid".into()))?;
    let transport = parse_transport(&parts)?;
    let security = super::vless::parse_security_pub(&parts)?;
    let traffic = parts.get("userinfo").and_then(parse_userinfo);
    let name = super::remark(&fragment.map(super::percent_decode), &parts.host, port);
    Ok(make_server(
        scope,
        raw,
        parts.host,
        port,
        Credentials::Vmess {
            uuid,
            alter_id: 0,
            cipher: None,
        },
        transport,
        security,
        name,
        traffic,
    ))
}

fn parse_json(rest: &str, raw: &str, scope: &str) -> Result<Server> {
    let (body, fragment) = super::split_fragment(rest);
    let decoded =
        decode_base64_any(body).ok_or_else(|| Error::Parse("vmess: invalid base64".into()))?;
    let text = String::from_utf8(decoded)
        .map_err(|_| Error::Parse("vmess: payload is not UTF-8".into()))?;
    let json: Value =
        serde_json::from_str(text.trim()).map_err(|e| Error::Parse(format!("vmess json: {e}")))?;
    let obj = json
        .as_object()
        .ok_or_else(|| Error::Parse("vmess json is not an object".into()))?;

    let address = get_str(obj.get("add")).ok_or_else(|| Error::Parse("vmess: no add".into()))?;
    let port = get_u16(obj.get("port")).ok_or_else(|| Error::Parse("vmess: no port".into()))?;
    let uuid = get_str(obj.get("id")).ok_or_else(|| Error::Parse("vmess: no id".into()))?;
    let alter_id = get_u32(obj.get("aid")).unwrap_or(0);
    let cipher = get_str(obj.get("scy"))
        .filter(|s| !s.is_empty() && !s.eq_ignore_ascii_case("auto"))
        .map(|s| s.to_ascii_lowercase());

    let net = get_str(obj.get("net"))
        .unwrap_or_default()
        .to_ascii_lowercase();
    let network = match net.as_str() {
        "ws" => Network::Ws,
        "grpc" => Network::Grpc,
        "httpupgrade" => Network::HttpUpgrade,
        "xhttp" | "splithttp" => Network::Xhttp,
        "h2" => Network::H2,
        "kcp" => Network::Kcp,
        "quic" => Network::Quic,
        _ => Network::Tcp,
    };
    let type_field = get_str(obj.get("type")).filter(|s| !s.is_empty() && s != "none");
    let path = get_str(obj.get("path")).filter(|s| !s.is_empty());
    let host = get_str(obj.get("host")).filter(|s| !s.is_empty());

    let mut transport = Transport {
        network,
        path: None,
        host,
        service_name: None,
        mode: None,
        header_type: None,
        seed: None,
        extra: None,
    };
    match network {
        Network::Grpc => {
            transport.service_name = path;
            transport.mode = type_field;
        }
        _ => {
            transport.path = path;
            transport.header_type = type_field;
        }
    }
    if let Some(extra) = get_str(obj.get("extra")).filter(|s| !s.is_empty()) {
        transport.extra = serde_json::from_str(&extra).ok();
    }

    let security = parse_json_security(obj);
    let traffic = obj
        .get("userinfo")
        .and_then(|v| v.as_str())
        .and_then(parse_userinfo);
    let name = obj
        .get("ps")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .filter(|s| !s.trim().is_empty())
        .or_else(|| fragment.map(str::to_string))
        .unwrap_or_else(|| format!("{address}:{port}"));

    Ok(make_server(
        scope,
        raw,
        address,
        port,
        Credentials::Vmess {
            uuid,
            alter_id,
            cipher,
        },
        transport,
        security,
        name,
        traffic,
    ))
}

fn parse_json_security(obj: &serde_json::Map<String, Value>) -> Security {
    let tls = get_str(obj.get("tls"))
        .unwrap_or_default()
        .to_ascii_lowercase();
    let sni = get_str(obj.get("sni")).filter(|s| !s.is_empty());
    let fp = get_str(obj.get("fp")).filter(|s| !s.is_empty());
    let alpn = obj
        .get("alpn")
        .map(|v| match v {
            Value::Array(_) => format!("{v}"),
            _ => get_str(Some(v)).unwrap_or_default(),
        })
        .map(|s| split_alpn(&s))
        .unwrap_or_default();
    match tls.as_str() {
        "reality" => Security::Reality {
            sni,
            public_key: get_str(obj.get("pbk")).unwrap_or_default(),
            short_id: get_str(obj.get("sid")).filter(|s| !s.is_empty()),
            spider_x: get_str(obj.get("spx")).filter(|s| !s.is_empty()),
            fingerprint: fp,
        },
        "tls" => Security::Tls {
            sni,
            alpn,
            fingerprint: fp,
            allow_insecure: get_str(obj.get("allowInsecure"))
                .map(|s| truthy(&s))
                .unwrap_or(false),
        },
        _ => Security::None,
    }
}

fn get_str(v: Option<&Value>) -> Option<String> {
    match v? {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

fn get_u32(v: Option<&Value>) -> Option<u32> {
    match v? {
        Value::Number(n) => n.as_u64().map(|x| x as u32),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

fn get_u16(v: Option<&Value>) -> Option<u16> {
    get_u32(v).and_then(|n| u16::try_from(n).ok())
}

/// Export a vmess server as the base64 JSON (v2rayN) form.
pub(super) fn to_link(server: &Server) -> String {
    let (uuid, alter_id, cipher) = match &server.credentials {
        Credentials::Vmess {
            uuid,
            alter_id,
            cipher,
        } => (uuid.clone(), *alter_id, cipher.clone()),
        _ => (String::new(), 0, None),
    };
    let t = &server.transport;
    let (tls, sni, pbk, sid, spx, alpn, fp) = match &server.security {
        Security::None => (
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
        ),
        Security::Tls {
            sni,
            alpn,
            fingerprint,
            ..
        } => (
            "tls".to_string(),
            sni.clone().unwrap_or_default(),
            String::new(),
            String::new(),
            String::new(),
            alpn.join(","),
            fingerprint.clone().unwrap_or_default(),
        ),
        Security::Reality {
            sni,
            public_key,
            short_id,
            spider_x,
            fingerprint,
        } => (
            "reality".to_string(),
            sni.clone().unwrap_or_default(),
            public_key.clone(),
            short_id.clone().unwrap_or_default(),
            spider_x.clone().unwrap_or_default(),
            String::new(),
            fingerprint.clone().unwrap_or_default(),
        ),
    };

    let (path_field, type_field) = match t.network {
        Network::Grpc => (
            t.service_name.clone().unwrap_or_default(),
            t.mode.clone().unwrap_or_else(|| "gun".to_string()),
        ),
        _ => (
            t.path.clone().unwrap_or_default(),
            t.header_type.clone().unwrap_or_else(|| "none".to_string()),
        ),
    };

    let mut obj = serde_json::Map::new();
    obj.insert("v".into(), Value::String("2".into()));
    obj.insert("ps".into(), Value::String(server.name.clone()));
    obj.insert("add".into(), Value::String(server.address.clone()));
    obj.insert("port".into(), Value::String(server.port.to_string()));
    obj.insert("id".into(), Value::String(uuid));
    obj.insert("aid".into(), Value::String(alter_id.to_string()));
    obj.insert(
        "scy".into(),
        Value::String(cipher.unwrap_or_else(|| "auto".to_string())),
    );
    obj.insert("net".into(), Value::String(t.network.as_str().into()));
    obj.insert("type".into(), Value::String(type_field));
    obj.insert(
        "host".into(),
        Value::String(t.host.clone().unwrap_or_default()),
    );
    obj.insert("path".into(), Value::String(path_field));
    obj.insert("tls".into(), Value::String(tls));
    if !sni.is_empty() {
        obj.insert("sni".into(), Value::String(sni));
    }
    if !alpn.is_empty() {
        obj.insert("alpn".into(), Value::String(alpn));
    }
    if !fp.is_empty() {
        obj.insert("fp".into(), Value::String(fp));
    }
    if !pbk.is_empty() {
        obj.insert("pbk".into(), Value::String(pbk));
    }
    if !sid.is_empty() {
        obj.insert("sid".into(), Value::String(sid));
    }
    if !spx.is_empty() {
        obj.insert("spx".into(), Value::String(spx));
    }
    if let Some(extra) = &t.extra {
        obj.insert("extra".into(), Value::String(extra.to_string()));
    }
    if let Some(traffic) = &server.traffic {
        obj.insert("userinfo".into(), Value::String(format_userinfo(traffic)));
    }

    let json = Value::Object(obj).to_string();
    use base64::Engine;
    let encoded = base64::engine::general_purpose::STANDARD.encode(json.as_bytes());
    format!("vmess://{encoded}")
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
    fn vmess_json_string_ports() {
        let json = r#"{"v":"2","ps":"Node","add":"v2.example.com","port":"443","id":"11111111-2222-3333-4444-555555555555","aid":"0","scy":"auto","net":"ws","type":"none","host":"v2.example.com","path":"/ray","tls":"tls","sni":"v2.example.com"}"#;
        let link = format!("vmess://{}", b64(json));
        let s = parse_link(&link, "local").unwrap();
        assert_eq!(s.address, "v2.example.com");
        assert_eq!(s.port, 443);
        assert_eq!(s.transport.network, Network::Ws);
        assert_eq!(s.transport.path.as_deref(), Some("/ray"));
        assert_eq!(s.name, "Node");
        match &s.credentials {
            Credentials::Vmess {
                uuid,
                alter_id,
                cipher,
            } => {
                assert_eq!(uuid, "11111111-2222-3333-4444-555555555555");
                assert_eq!(alter_id.to_owned(), 0);
                assert!(cipher.is_none());
            }
            _ => panic!(),
        }
        assert!(matches!(s.security, Security::Tls { .. }));
    }

    #[test]
    fn vmess_json_numeric_and_grpc() {
        let json = r#"{"v":2,"ps":"GRPC","add":"g.example.com","port":8443,"id":"abc","aid":64,"scy":"chacha20-poly1305","net":"grpc","type":"multi","host":"","path":"svc","tls":"tls","sni":"g.example.com","alpn":"h2"}"#;
        let link = format!("vmess://{}", b64(json));
        let s = parse_link(&link, "local").unwrap();
        assert_eq!(s.port, 8443);
        assert_eq!(s.transport.network, Network::Grpc);
        assert_eq!(s.transport.service_name.as_deref(), Some("svc"));
        assert_eq!(s.transport.mode.as_deref(), Some("multi"));
        match &s.credentials {
            Credentials::Vmess {
                alter_id, cipher, ..
            } => {
                assert_eq!(alter_id.to_owned(), 64);
                assert_eq!(cipher.as_deref(), Some("chacha20-poly1305"));
            }
            _ => panic!(),
        }
    }

    #[test]
    fn vmess_uri_form() {
        let link = "vmess://22222222-3333-4444-5555-666666666666@uri.example.com:2053?type=ws&security=tls&path=%2Fv&host=uri.example.com&sni=uri.example.com#URI%20Node";
        let s = parse_link(link, "local").unwrap();
        assert_eq!(s.address, "uri.example.com");
        assert_eq!(s.transport.network, Network::Ws);
        assert_eq!(s.name, "URI Node");
    }

    #[test]
    fn vmess_roundtrip() {
        let link = "vmess://44444444-5555-6666-7777-888888888888@rt.example.com:443?type=grpc&security=tls&serviceName=rt&mode=gun&sni=rt.example.com#RT";
        let a = parse_link(link, "local").unwrap();
        let b = parse_link(&to_link(&a), "local").unwrap();
        assert_eq!(a.name, b.name);
        assert_eq!(a.address, b.address);
        assert_eq!(a.port, b.port);
        assert_eq!(a.credentials, b.credentials);
        assert_eq!(a.transport, b.transport);
        assert_eq!(a.security, b.security);
    }
}

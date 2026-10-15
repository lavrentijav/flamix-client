//! Share links <-> [`Server`]. OWNER: task T01.
//!
//! Supported schemes: `vless://`, `vmess://` (v2rayN base64 JSON and the URI form),
//! `trojan://`, `ss://` (SIP002 + legacy base64), `hysteria2://` / `hy2://`, `tuic://`,
//! `wireguard://` / `wg://`, `socks://` / `socks5://`, `http://` / `https://` (proxy links).

mod hysteria2;
mod socks_http;
mod ss;
mod tuic;
mod vless;
mod vmess;
mod wireguard;

use crate::error::Error;
use crate::model::{Credentials, Security, Server, TrafficInfo, Transport};
use crate::Result;

/// Parse one share link. `scope` is passed to [`Server::compute_id`]
/// (subscription id or "local"). Leading/trailing whitespace is ignored.
pub fn parse_link(link: &str, scope: &str) -> Result<Server> {
    let raw = link.trim();
    let (scheme, rest) =
        split_scheme(raw).ok_or_else(|| Error::Parse(format!("missing scheme in {raw:?}")))?;
    match scheme.as_str() {
        "vless" => vless::parse(rest, raw, scope, false),
        "trojan" => vless::parse(rest, raw, scope, true),
        "vmess" => vmess::parse(rest, raw, scope),
        "ss" => ss::parse(rest, raw, scope),
        "hysteria2" | "hy2" => hysteria2::parse(rest, raw, scope),
        "tuic" => tuic::parse(rest, raw, scope),
        "wireguard" | "wg" => wireguard::parse(rest, raw, scope),
        "socks" | "socks5" | "http" | "https" => socks_http::parse(&scheme, rest, raw, scope),
        other => Err(Error::Unsupported(format!("scheme {other}"))),
    }
}

/// Parse many links separated by newlines (also `\r\n`, spaces are not separators).
/// Lines that fail to parse are skipped and reported in the second element as (line, error).
pub fn parse_links(text: &str, scope: &str) -> (Vec<Server>, Vec<(String, String)>) {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut servers = Vec::new();
    let mut errors = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        match parse_link(line, scope) {
            Ok(server) => servers.push(server),
            Err(err) => errors.push((line.to_string(), err.to_string())),
        }
    }
    (servers, errors)
}

/// Export a server back to a share link (round-trips with [`parse_link`]).
pub fn to_link(server: &Server) -> String {
    match &server.credentials {
        Credentials::Vless { .. } => vless::to_link(server, false),
        Credentials::Trojan { .. } => vless::to_link(server, true),
        Credentials::Vmess { .. } => vmess::to_link(server),
        Credentials::Shadowsocks { .. } => ss::to_link(server),
        Credentials::Hysteria2 { .. } => hysteria2::to_link(server),
        Credentials::Tuic { .. } => tuic::to_link(server),
        Credentials::Wireguard { .. } => wireguard::to_link(server),
        Credentials::Socks { .. } | Credentials::Http { .. } => socks_http::to_link(server),
    }
}

// ---------------------------------------------------------------------------
// Shared parsing helpers
// ---------------------------------------------------------------------------

/// Split `scheme://rest`, lower-casing the scheme.
pub(super) fn split_scheme(link: &str) -> Option<(String, &str)> {
    let idx = link.find("://")?;
    let scheme = link[..idx].to_ascii_lowercase();
    if scheme.is_empty() {
        return None;
    }
    Some((scheme, &link[idx + 3..]))
}

/// Percent-decode a string (UTF-8 lossy). Does not treat `+` as space, so base64
/// values survive untouched.
pub(super) fn percent_decode(s: &str) -> String {
    percent_encoding::percent_decode_str(s)
        .decode_utf8_lossy()
        .into_owned()
}

/// Decode base64 trying standard / url-safe alphabets, with and without padding.
pub(super) fn decode_base64_any(s: &str) -> Option<Vec<u8>> {
    use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD, URL_SAFE, URL_SAFE_NO_PAD};
    use base64::Engine;
    let cleaned: String = s.chars().filter(|c| !c.is_ascii_whitespace()).collect();
    if cleaned.is_empty() {
        return None;
    }
    let bytes = cleaned.as_bytes();
    STANDARD
        .decode(bytes)
        .ok()
        .or_else(|| STANDARD_NO_PAD.decode(bytes).ok())
        .or_else(|| URL_SAFE.decode(bytes).ok())
        .or_else(|| URL_SAFE_NO_PAD.decode(bytes).ok())
}

/// Common URL parts shared by most schemes.
#[derive(Debug, Clone)]
pub(super) struct Parts {
    /// Percent-decoded userinfo (before the last `@`).
    pub userinfo: Option<String>,
    pub host: String,
    /// Raw port text (may contain a comma/range for hysteria2), not yet parsed.
    pub port_raw: Option<String>,
    pub query: Vec<(String, String)>,
    pub fragment: Option<String>,
}

impl Parts {
    pub fn get(&self, key: &str) -> Option<&str> {
        self.query
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(key))
            .map(|(_, v)| v.as_str())
    }

    pub fn port(&self) -> Result<u16> {
        parse_port(self.port_raw.as_deref().unwrap_or(""))
            .ok_or_else(|| Error::Parse("missing or invalid port".into()))
    }

    pub fn remark(&self, port: u16) -> String {
        remark(&self.fragment, &self.host, port)
    }
}

/// Parse `userinfo@host:port?query#fragment` (query/fragment optional).
pub(super) fn parse_parts(rest: &str) -> Result<Parts> {
    let (rest, fragment) = split_fragment(rest);
    let (authority, query) = split_query(rest);
    let (userinfo, hostport) = match authority.rsplit_once('@') {
        Some((u, h)) => (Some(percent_decode(u)), h),
        None => (None, authority),
    };
    if hostport.is_empty() {
        return Err(Error::Parse("missing host".into()));
    }
    let (host, port_raw) = split_host_port(hostport)?;
    if host.is_empty() {
        return Err(Error::Parse("empty host".into()));
    }
    Ok(Parts {
        userinfo,
        host,
        port_raw,
        query: parse_query(query.unwrap_or("")),
        fragment: fragment.map(percent_decode),
    })
}

/// Split off the `#fragment` (first `#`).
pub(super) fn split_fragment(s: &str) -> (&str, Option<&str>) {
    match s.split_once('#') {
        Some((a, b)) => (a, Some(b)),
        None => (s, None),
    }
}

/// Split off the `?query` (first `?`).
pub(super) fn split_query(s: &str) -> (&str, Option<&str>) {
    match s.split_once('?') {
        Some((a, b)) => (a, Some(b)),
        None => (s, None),
    }
}

/// Split `host:port`, honouring `[IPv6]`, bare IPv6 (no port) and raw port text.
pub(super) fn split_host_port(s: &str) -> Result<(String, Option<String>)> {
    if let Some(rest) = s.strip_prefix('[') {
        let end = rest
            .find(']')
            .ok_or_else(|| Error::Parse("unterminated IPv6 host".into()))?;
        let host = rest[..end].to_string();
        let after = &rest[end + 1..];
        let port = if after.is_empty() {
            None
        } else {
            Some(
                after
                    .strip_prefix(':')
                    .ok_or_else(|| Error::Parse("bad host suffix".into()))?
                    .to_string(),
            )
        };
        Ok((host, port))
    } else if let Some((h, p)) = s.rsplit_once(':') {
        if h.contains(':') {
            Ok((s.to_string(), None))
        } else {
            Ok((h.to_string(), Some(p.to_string())))
        }
    } else {
        Ok((s.to_string(), None))
    }
}

/// Parse a query string into decoded key/value pairs.
pub(super) fn parse_query(q: &str) -> Vec<(String, String)> {
    q.split('&')
        .filter(|s| !s.is_empty())
        .map(|pair| match pair.split_once('=') {
            Some((k, v)) => (percent_decode(k), percent_decode(v)),
            None => (percent_decode(pair), String::new()),
        })
        .collect()
}

/// Parse a plain decimal port.
pub(super) fn parse_port(s: &str) -> Option<u16> {
    s.trim().parse::<u16>().ok()
}

/// Default remark: the fragment if non-empty, else `host:port`.
pub(super) fn remark(fragment: &Option<String>, host: &str, port: u16) -> String {
    match fragment {
        Some(name) if !name.trim().is_empty() => name.trim().to_string(),
        _ => format!("{host}:{port}"),
    }
}

/// Parse a `subscription-userinfo`-style value (`upload=1; download=2; ...`).
/// Private duplicate of the T02 parser — see `docs/subscription-headers.md` (Extension 2 b).
pub(super) fn parse_userinfo(value: &str) -> Option<TrafficInfo> {
    let mut info = TrafficInfo::default();
    let mut found = false;
    for part in value.split(';') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let Some((key, val)) = part.split_once('=') else {
            continue;
        };
        let val = val.trim();
        match key.trim().to_ascii_lowercase().as_str() {
            "upload" => {
                if let Ok(n) = val.parse() {
                    info.upload = n;
                    found = true;
                }
            }
            "download" => {
                if let Ok(n) = val.parse() {
                    info.download = n;
                    found = true;
                }
            }
            "total" => {
                if let Ok(n) = val.parse() {
                    info.total = Some(n);
                    found = true;
                }
            }
            "expire" => {
                if let Ok(n) = val.parse() {
                    info.expire = Some(n);
                    found = true;
                }
            }
            _ => {}
        }
    }
    found.then_some(info)
}

/// Build a [`Server`] from parsed pieces, computing id / country / summary.
#[allow(clippy::too_many_arguments)]
pub(super) fn make_server(
    scope: &str,
    raw: &str,
    address: String,
    port: u16,
    credentials: Credentials,
    transport: Transport,
    security: Security,
    name: String,
    traffic: Option<TrafficInfo>,
) -> Server {
    let id = Server::compute_id(scope, &address, port, &credentials);
    let country = crate::country::detect_country(&name);
    Server {
        id,
        name,
        country,
        address,
        port,
        credentials,
        transport,
        security,
        raw_link: Some(raw.to_string()),
        traffic,
        local_traffic: TrafficInfo::default(),
        last_ping_ms: None,
    }
}

/// Query string builder that percent-encodes unsafe characters.
pub(super) struct Query {
    pairs: Vec<(String, String)>,
}

impl Default for Query {
    fn default() -> Self {
        Self::new()
    }
}

impl Query {
    pub fn new() -> Self {
        Self { pairs: Vec::new() }
    }

    pub fn add(&mut self, key: &str, value: &str) {
        if value.is_empty() {
            return;
        }
        self.pairs.push((key.to_string(), value.to_string()));
    }

    pub fn add_owned(&mut self, key: &str, value: String) {
        self.add(key, &value);
    }

    pub fn add_flag(&mut self, key: &str, on: bool) {
        if on {
            self.pairs.push((key.to_string(), "1".to_string()));
        }
    }

    pub fn finish(&self) -> String {
        const SET: &percent_encoding::AsciiSet = &percent_encoding::CONTROLS
            .add(b' ')
            .add(b'"')
            .add(b'#')
            .add(b'&')
            .add(b'+')
            .add(b'<')
            .add(b'=')
            .add(b'>')
            .add(b'?')
            .add(b'%')
            .add(b'`')
            .add(b'{')
            .add(b'}');
        self.pairs
            .iter()
            .map(|(k, v)| {
                let encoded = percent_encoding::utf8_percent_encode(v, SET).to_string();
                format!("{k}={encoded}")
            })
            .collect::<Vec<_>>()
            .join("&")
    }
}

/// Format `host:port`, wrapping bare IPv6 addresses in brackets.
pub(super) fn format_host_port(address: &str, port: u16) -> String {
    if address.contains(':') {
        format!("[{address}]:{port}")
    } else {
        format!("{address}:{port}")
    }
}

/// Percent-encode a userinfo component for a link (keeps `:` for tuic's `uuid:password`).
pub(super) fn encode_userinfo(s: &str) -> String {
    const SET: &percent_encoding::AsciiSet = &percent_encoding::CONTROLS
        .add(b' ')
        .add(b'@')
        .add(b'/')
        .add(b'?')
        .add(b'#')
        .add(b'%')
        .add(b'&')
        .add(b'=')
        .add(b'+');
    percent_encoding::utf8_percent_encode(s, SET).to_string()
}

/// Percent-encode a display name for the `#remark` part.
pub(super) fn encode_remark(name: &str) -> String {
    const SET: &percent_encoding::AsciiSet = &percent_encoding::CONTROLS
        .add(b' ')
        .add(b'#')
        .add(b'%')
        .add(b'&')
        .add(b'?');
    percent_encoding::utf8_percent_encode(name, SET).to_string()
}

/// Append `?query` and `#remark` to a base link.
pub(super) fn finish_link(base: String, query: &Query, name: &str) -> String {
    let mut link = base;
    let q = query.finish();
    if !q.is_empty() {
        link.push('?');
        link.push_str(&q);
    }
    if !name.is_empty() {
        link.push('#');
        link.push_str(&encode_remark(name));
    }
    link
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_variants() {
        assert_eq!(decode_base64_any("aGVsbG8=").unwrap(), b"hello");
        assert_eq!(decode_base64_any("aGVsbG8").unwrap(), b"hello");
        assert!(decode_base64_any("!!!not base64!!!").is_none());
        assert!(decode_base64_any("   ").is_none());
    }

    #[test]
    fn host_port_forms() {
        assert_eq!(
            split_host_port("[::1]:443").unwrap(),
            ("::1".into(), Some("443".into()))
        );
        assert_eq!(
            split_host_port("example.com:8443").unwrap(),
            ("example.com".into(), Some("8443".into()))
        );
        assert_eq!(split_host_port("::1").unwrap(), ("::1".into(), None));
    }

    #[test]
    fn userinfo_parser() {
        let t = parse_userinfo("upload=1; download=2; total=3; expire=4").unwrap();
        assert_eq!(t.upload, 1);
        assert_eq!(t.download, 2);
        assert_eq!(t.total, Some(3));
        assert_eq!(t.expire, Some(4));
        assert!(parse_userinfo("nonsense").is_none());
    }

    fn strip(mut server: Server) -> Server {
        server.raw_link = None;
        server
    }

    #[test]
    fn roundtrip_all_protocols() {
        let links = [
            "vless://uuid-1@de1.example.com:443?type=tcp&security=reality&sni=www.microsoft.com&fp=chrome&pbk=PBK123&sid=abcd&spx=%2F&flow=xtls-rprx-vision#%F0%9F%87%A9%F0%9F%87%AA%20Germany",
            "vless://uuid-1@example.com:8443?type=ws&security=tls&sni=example.com&host=example.com&path=%2Fws&fp=firefox&alpn=h2,http%2F1.1&encryption=none#Node",
            "vless://uuid-1@example.com:443?type=grpc&security=tls&serviceName=svc&mode=gun&sni=example.com#grpc",
            "vless://uuid-1@example.com:443?type=xhttp&security=tls&path=%2Fx&host=h.com&mode=packet-up&extra=%7B%22xPaddingBytes%22%3A%22100-1000%22%7D#xhttp",
            "trojan://pass%40word@example.com:443?sni=example.com&type=ws&path=%2Ftj#Trojan",
            "vmess://aGVsbG8@example.com:443?type=ws&security=tls&path=%2Fv&sni=example.com#VM",
            "ss://YWVzLTI1Ni1nY206c2VjcmV0@ss.example.com:8388/?plugin=obfs-local%3Bobfs%3Dhttp#SS",
            "ss://YWVzLTI1Ni1nY206cHdAMS4yLjMuNDo4Mzg4#legacy",
            "hysteria2://pw@hy.example.com:443,20000-30000?sni=hy.example.com&insecure=1&obfs=salamander&obfs-password=x&alpn=h3&mport=20000-30000#HY",
            "tuic://u:p%40w@tuic.example.com:443?congestion_control=bbr&udp_relay_mode=native&alpn=h3&sni=tuic.example.com#TUIC",
            "wireguard://a2V5@wg.example.com:51820?publickey=cHVi&presharedkey=cHNr&address=10.0.0.2%2F32&reserved=1,2,3&mtu=1280#WG",
            "socks5://user:pass@127.0.0.1:1080#SOCKS",
            "https://u:p@proxy.example.com:8443#HTTPS",
            "http://proxy.example.com:8080#HTTP",
        ];
        for link in links {
            let a = parse_link(link, "local").unwrap_or_else(|e| panic!("{link}: {e}"));
            let b =
                parse_link(&to_link(&a), "local").unwrap_or_else(|e| panic!("export {link}: {e}"));
            assert_eq!(strip(a), strip(b), "round-trip mismatch for {link}");
        }
    }

    #[test]
    fn vmess_json_roundtrip_real() {
        use base64::Engine;
        let json = r#"{"v":"2","ps":"Node","add":"v2.example.com","port":"443","id":"11111111-2222-3333-4444-555555555555","aid":"0","scy":"auto","net":"ws","type":"none","host":"v2.example.com","path":"/ray","tls":"tls","sni":"v2.example.com","alpn":"h2"}"#;
        let encoded = base64::engine::general_purpose::STANDARD.encode(json.as_bytes());
        let a = parse_link(&format!("vmess://{encoded}"), "local").unwrap();
        let b = parse_link(&to_link(&a), "local").unwrap();
        assert_eq!(strip(a), strip(b));
    }

    #[test]
    fn parse_links_skips_comments_bom_and_reports_errors() {
        let text = "\u{feff}#profile-title: test\n\nvless://u@a.example.com:443?security=none#A\nbad://x\ntrojan://p@b.example.com:443#B\n";
        let (servers, errors) = parse_links(text, "sub");
        assert_eq!(servers.len(), 2);
        assert_eq!(servers[0].name, "A");
        assert_eq!(servers[1].name, "B");
        assert_eq!(errors.len(), 1);
        assert!(errors[0].1.contains("unsupported"));
    }

    #[test]
    fn unsupported_vs_parse_errors() {
        assert!(matches!(
            parse_link("unknownscheme://x", "local"),
            Err(crate::Error::Unsupported(_))
        ));
        assert!(matches!(
            parse_link("vless://", "local"),
            Err(crate::Error::Parse(_))
        ));
        assert!(matches!(
            parse_link("vless://uuid@host-no-port", "local"),
            Err(crate::Error::Parse(_))
        ));
    }

    #[test]
    fn userinfo_extension() {
        let s = parse_link(
            "vless://u@a.example.com:443?security=none&userinfo=download%3D5368709120%3Btotal%3D53687091200#A",
            "local",
        )
        .unwrap();
        let t = s.traffic.unwrap();
        assert_eq!(t.download, 5368709120);
        assert_eq!(t.total, Some(53687091200));
    }

    #[test]
    fn garbage_never_panics() {
        for s in [
            "",
            "://",
            "vless://",
            "vless://@@@",
            "vmess://!!!!",
            "ss://!!!!",
            "trojan://",
            "hysteria2://",
            "wireguard://",
            "socks://",
            "\u{feff}",
        ] {
            let _ = parse_link(s, "local");
        }
    }
}

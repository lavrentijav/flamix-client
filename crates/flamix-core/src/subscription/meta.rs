//! Subscription metadata: `subscription-userinfo`, `profile-*` headers, `#key: value` body lines.

use super::util::decode_b64_str;
use super::SubscriptionMeta;
use crate::model::TrafficInfo;
use percent_encoding::percent_decode_str;
use std::collections::HashMap;

/// Parse a `subscription-userinfo` style value: `upload=1; download=2; total=3; expire=4`.
///
/// Keys may appear in any order, separated by `;` or `,`, spaces are optional, unknown keys
/// are ignored and numbers may be floats (`1.5e9`, truncated). Returns `None` when no known
/// key was parsed.
pub fn parse_userinfo(value: &str) -> Option<TrafficInfo> {
    let mut info = TrafficInfo::default();
    let mut found = false;
    for part in value.split([';', ',']) {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let Some((key, val)) = part.split_once('=') else {
            continue;
        };
        let Some(n) = parse_number(val) else {
            continue;
        };
        match key.trim().to_ascii_lowercase().as_str() {
            "upload" => {
                info.upload = n;
                found = true;
            }
            "download" => {
                info.download = n;
                found = true;
            }
            "total" => {
                info.total = Some(n);
                found = true;
            }
            "expire" => {
                info.expire = Some(n as i64);
                found = true;
            }
            _ => {}
        }
    }
    found.then_some(info)
}

fn parse_number(value: &str) -> Option<u64> {
    let f: f64 = value.trim().parse().ok()?;
    if !f.is_finite() || f < 0.0 {
        return None;
    }
    Some(f as u64)
}

/// Extract metadata from headers (names case-insensitive) and the body.
/// Header values win over `#key: value` body lines when both are present.
pub fn parse_meta(headers: &[(String, String)], body: &str) -> SubscriptionMeta {
    let mut meta = SubscriptionMeta::default();

    let mut body_title = None;
    for (key, value) in body_metadata(body) {
        let key = key.to_ascii_lowercase();
        if key == "profile-title" || key == "title" {
            if body_title.is_none() {
                body_title = decode_text(&value);
            }
        } else {
            apply_field(&mut meta, &key, &value, false);
        }
    }

    let mut header_title = None;
    let mut disposition_title = None;
    for (key, value) in headers {
        let key = key.to_ascii_lowercase();
        match key.as_str() {
            "profile-title" | "title" => {
                if let Some(t) = decode_text(value) {
                    header_title = Some(t);
                }
            }
            "content-disposition" => {
                if disposition_title.is_none() {
                    disposition_title = filename(value);
                }
            }
            _ => apply_field(&mut meta, &key, value, true),
        }
    }

    meta.title = header_title.or(disposition_title).or(body_title);
    meta
}

fn apply_field(meta: &mut SubscriptionMeta, key: &str, value: &str, header: bool) {
    match key {
        "subscription-userinfo" | "userinfo" => {
            if let Some(info) = parse_userinfo(value) {
                meta.userinfo = Some(info);
            }
        }
        "profile-update-interval" | "update-interval" => {
            if let Some(n) = parse_interval(value) {
                meta.update_interval_hours = Some(n);
            }
        }
        "support-url" => {
            if let Some(v) = non_empty(value) {
                meta.support_url = Some(v);
            }
        }
        "profile-web-page-url" | "web-page-url" => {
            if let Some(v) = non_empty(value) {
                meta.web_page_url = Some(v);
            }
        }
        "announce" => {
            if let Some(v) = decode_text(value) {
                meta.announce = Some(v);
            }
        }
        "profile-backup-urls" | "backup-urls" | "x-backup-urls" | "fallback-urls" => {
            let urls = parse_backup_urls(value);
            if header {
                if !urls.is_empty() {
                    meta.backup_urls = urls;
                }
            } else {
                meta.backup_urls.extend(urls);
            }
        }
        "server-userinfo" | "profile-server-userinfo" => {
            for (k, v) in parse_server_userinfo(value) {
                meta.server_userinfo.insert(k, v);
            }
        }
        _ => {}
    }
}

/// Parse the `server-userinfo` value: a JSON object mapping names / `host:port` /
/// credential ids to either a userinfo string or an object with the same keys.
/// The value may be plain JSON or `base64:<b64>`.
pub fn parse_server_userinfo(value: &str) -> HashMap<String, TrafficInfo> {
    let text = decode_text(value).unwrap_or_else(|| value.trim().to_string());
    let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) else {
        return HashMap::new();
    };
    let Some(obj) = json.as_object() else {
        return HashMap::new();
    };
    let mut map = HashMap::new();
    for (key, value) in obj {
        let info = match value {
            serde_json::Value::String(s) => parse_userinfo(s),
            _ => traffic_from_json(value),
        };
        if let Some(info) = info {
            map.insert(key.clone(), info);
        }
    }
    map
}

fn traffic_from_json(value: &serde_json::Value) -> Option<TrafficInfo> {
    let obj = value.as_object()?;
    let mut info = TrafficInfo::default();
    let mut found = false;
    for (key, value) in obj {
        let n = match value {
            serde_json::Value::Number(n) => n.as_f64(),
            serde_json::Value::String(s) => s.trim().parse::<f64>().ok(),
            _ => None,
        };
        let Some(n) = n.filter(|n| n.is_finite() && *n >= 0.0) else {
            continue;
        };
        let n = n as u64;
        match key.to_ascii_lowercase().as_str() {
            "upload" => {
                info.upload = n;
                found = true;
            }
            "download" => {
                info.download = n;
                found = true;
            }
            "total" => {
                info.total = Some(n);
                found = true;
            }
            "expire" => {
                info.expire = Some(n as i64);
                found = true;
            }
            _ => {}
        }
    }
    found.then_some(info)
}

/// Decode a `base64:<b64>` value, otherwise return the trimmed text. Empty values are `None`.
fn decode_text(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    if value.len() >= 7 {
        if let Some(prefix) = value.get(..7) {
            if prefix.eq_ignore_ascii_case("base64:") {
                return decode_b64_str(&value[7..]).map(|s| s.trim().to_string());
            }
        }
    }
    Some(value.to_string())
}

fn non_empty(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_string())
}

fn parse_interval(value: &str) -> Option<u32> {
    value.trim().parse::<u32>().ok()
}

fn parse_backup_urls(value: &str) -> Vec<String> {
    let text = decode_text(value).unwrap_or_else(|| value.to_string());
    text.split(|c: char| c == ',' || c.is_whitespace())
        .map(str::trim)
        .filter(|u| !u.is_empty())
        .filter(|u| is_http_url(u))
        .map(str::to_string)
        .collect()
}

fn is_http_url(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    lower.starts_with("http://") || lower.starts_with("https://")
}

/// Leading block of `#key: value` lines in the (possibly base64 encoded) body.
fn body_metadata(body: &str) -> Vec<(String, String)> {
    let decoded = decode_body_for_meta(body);
    let mut out = Vec::new();
    for line in decoded.lines() {
        let line = line.trim_end_matches('\r');
        let Some(rest) = line.strip_prefix('#') else {
            break;
        };
        let rest = rest.trim_start_matches('#').trim();
        if let Some((key, value)) = rest.split_once(':') {
            out.push((key.trim().to_string(), value.trim().to_string()));
        }
    }
    out
}

fn decode_body_for_meta(body: &str) -> std::borrow::Cow<'_, str> {
    if let Some(decoded) = decode_b64_str(body) {
        if decoded.contains("://") {
            return std::borrow::Cow::Owned(decoded);
        }
    }
    std::borrow::Cow::Borrowed(body)
}

/// RFC 6266 `filename` / `filename*=UTF-8''..` from a `Content-Disposition` value.
fn filename(disposition: &str) -> Option<String> {
    let lower = disposition.to_ascii_lowercase();
    if let Some(idx) = lower.find("filename*=") {
        let rest = disposition[idx + "filename*=".len()..]
            .trim()
            .trim_matches('"');
        if let Some((_charset, tail)) = rest.split_once('\'') {
            if let Some((_lang, encoded)) = tail.split_once('\'') {
                let decoded = percent_decode_str(encoded).decode_utf8_lossy();
                let decoded = decoded.trim();
                if !decoded.is_empty() {
                    return Some(decoded.to_string());
                }
            }
        }
    }
    if let Some(idx) = lower.find("filename=") {
        let rest = disposition[idx + "filename=".len()..].trim_start();
        let raw = if let Some(stripped) = rest.strip_prefix('"') {
            stripped.split('"').next().unwrap_or("")
        } else {
            rest.split(';').next().unwrap_or("").trim()
        };
        if !raw.is_empty() {
            return Some(raw.to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(name: &str, value: &str) -> (String, String) {
        (name.to_string(), value.to_string())
    }

    #[test]
    fn userinfo_full() {
        let t =
            parse_userinfo("upload=0; download=1073741824; total=107374182400; expire=1798761600")
                .unwrap();
        assert_eq!(t.upload, 0);
        assert_eq!(t.download, 1_073_741_824);
        assert_eq!(t.total, Some(107_374_182_400));
        assert_eq!(t.expire, Some(1_798_761_600));
    }

    #[test]
    fn userinfo_variants() {
        let t = parse_userinfo("download=1.5e9; total=2").unwrap();
        assert_eq!(t.download, 1_500_000_000);
        assert_eq!(t.total, Some(2));
        assert_eq!(t.upload, 0);

        let t = parse_userinfo("upload = 5 ; total=0 ; expire=0").unwrap();
        assert_eq!(t.upload, 5);
        assert_eq!(t.total, Some(0));
        assert_eq!(t.expire, Some(0));

        let t = parse_userinfo("upload=1,download=2").unwrap();
        assert_eq!((t.upload, t.download), (1, 2));

        let t = parse_userinfo("foo=1; upload=3; bar=x").unwrap();
        assert_eq!(t.upload, 3);

        assert!(parse_userinfo("foo=1; bar=2").is_none());
        assert!(parse_userinfo("").is_none());
    }

    #[test]
    fn meta_headers_win_over_body() {
        let body = "#profile-title: Body title\n#support-url: https://body.example\n#subscription-userinfo: download=5\nvless://x@h:443#A\n";
        let b64_title = {
            use base64::engine::general_purpose::STANDARD;
            use base64::Engine;
            STANDARD.encode("My VPN")
        };
        let headers = vec![
            header("Profile-Title", &format!("base64:{b64_title}")),
            header("subscription-userinfo", "upload=1; download=2; total=100"),
        ];
        let meta = parse_meta(&headers, body);
        assert_eq!(meta.title.as_deref(), Some("My VPN"));
        assert_eq!(meta.support_url.as_deref(), Some("https://body.example"));
        assert_eq!(meta.userinfo.unwrap().download, 2);
    }

    #[test]
    fn meta_from_body_only() {
        let body = "#profile-title: Hello\n#profile-update-interval: 24\n#profile-backup-urls: https://a.example/s, https://b.example/s\n#server-userinfo: base64:dGVzdA==\nvless://x@h:443#A\n";
        let meta = parse_meta(&[], body);
        assert_eq!(meta.title.as_deref(), Some("Hello"));
        assert_eq!(meta.update_interval_hours, Some(24));
        assert_eq!(
            meta.backup_urls,
            vec!["https://a.example/s", "https://b.example/s"]
        );
    }

    #[test]
    fn meta_from_base64_body() {
        use base64::engine::general_purpose::STANDARD;
        use base64::Engine;
        let plain = "#profile-title: Hi\n#announce: base64:aGVsbG8=\nvless://x@h:443#A\n";
        let body = STANDARD.encode(plain);
        let meta = parse_meta(&[], &body);
        assert_eq!(meta.title.as_deref(), Some("Hi"));
        assert_eq!(meta.announce.as_deref(), Some("hello"));
    }

    #[test]
    fn meta_content_disposition_fallback() {
        let headers = vec![header(
            "Content-Disposition",
            "attachment; filename=\"plain.txt\"; filename*=UTF-8''real%20name.txt",
        )];
        let meta = parse_meta(&headers, "vless://x@h:443#A\n");
        assert_eq!(meta.title.as_deref(), Some("real name.txt"));

        let headers = vec![header(
            "content-disposition",
            "attachment; filename=\"fallback.txt\"",
        )];
        let meta = parse_meta(&headers, "vless://x@h:443#A\n");
        assert_eq!(meta.title.as_deref(), Some("fallback.txt"));
    }

    #[test]
    fn meta_server_userinfo_header_wins() {
        use base64::engine::general_purpose::STANDARD;
        use base64::Engine;
        let json = r#"{"A":"download=1; total=2","de1.example.com:443":{"upload":0,"download":5,"total":10}}"#;
        let value = format!("base64:{}", STANDARD.encode(json));
        let headers = vec![header("server-userinfo", &value)];
        let meta = parse_meta(&headers, "");
        assert_eq!(meta.server_userinfo.len(), 2);
        assert_eq!(meta.server_userinfo["A"].total, Some(2));
        assert_eq!(meta.server_userinfo["de1.example.com:443"].download, 5);
    }

    #[test]
    fn backup_urls_aliases_and_base64() {
        use base64::engine::general_purpose::STANDARD;
        use base64::Engine;
        let headers = vec![header(
            "x-backup-urls",
            &format!("base64:{}", STANDARD.encode("https://a/s\nhttps://b/s")),
        )];
        let meta = parse_meta(&headers, "");
        assert_eq!(meta.backup_urls, vec!["https://a/s", "https://b/s"]);
    }
}

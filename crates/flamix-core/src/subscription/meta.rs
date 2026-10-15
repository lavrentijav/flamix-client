//! Subscription metadata: `subscription-userinfo`, `profile-*` headers, `#key: value` body lines.

use super::util::decode_b64_str;
use super::SubscriptionMeta;
use crate::model::{HwidStatus, InfoBanner, ProviderRouting, Timestamp, TrafficInfo};
use crate::provider_routing::{self, RoutingLinkAction};
use percent_encoding::percent_decode_str;
use std::collections::HashMap;

/// Happ app-management keys (not otherwise mapped) stored verbatim in `app_overrides`.
const APP_OVERRIDE_KEYS: &[&str] = &[
    "custom-tunnel-config",
    "socks-auth-mode",
    "socks-auth-user",
    "socks-auth-password",
    "http-auth-mode",
    "http-auth-user",
    "http-auth-password",
    "tun-type",
    "new-url",
    "new-domain",
];

/// Response headers carrying HWID device-limit state (Remnawave / v2RayTun).
const HWID_HEADERS: &[&str] = &[
    "x-hwid-limit",
    "x-hwid-active",
    "x-hwid-max-devices-reached",
    "x-hwid-not-supported",
];

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
        // Happ `fallback-url` is an alias of `profile-backup-urls`.
        "profile-backup-urls"
        | "backup-urls"
        | "x-backup-urls"
        | "fallback-urls"
        | "fallback-url" => {
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
        "providerid" | "provider-id" => {
            if let Some(v) = non_empty(value) {
                meta.provider_id = Some(v);
            }
        }
        "routing" => {
            if value.trim().eq_ignore_ascii_case("happ://routing/off") {
                meta.routing_enable = Some(false);
            } else if let Some(routing) = parse_routing_value(value) {
                meta.routing = Some(routing);
            }
        }
        "routing-enable" => {
            if !value.trim().is_empty() {
                meta.routing_enable = Some(is_enabled(value));
            }
        }
        "announce-url" => {
            if let Some(v) = non_empty(value) {
                meta.announce_url = Some(v);
            }
        }
        "sub-info-text" => {
            let banner = meta.info_banner.get_or_insert_with(InfoBanner::default);
            let text = value.trim();
            banner.text = if text.is_empty() || text == "0" {
                String::new()
            } else {
                decode_text(value).unwrap_or_default()
            };
        }
        "sub-info-color" => {
            let banner = meta.info_banner.get_or_insert_with(InfoBanner::default);
            banner.color = non_empty(value);
        }
        "sub-info-button-text" => {
            let banner = meta.info_banner.get_or_insert_with(InfoBanner::default);
            banner.button_text = decode_text(value);
        }
        "sub-info-button-link" => {
            let banner = meta.info_banner.get_or_insert_with(InfoBanner::default);
            banner.button_link = non_empty(value);
        }
        "sub-expire" => {
            if let Some(v) = non_empty(value) {
                meta.expire_notice = Some(v);
            }
        }
        "sub-expire-button-link" => {
            if let Some(v) = non_empty(value) {
                meta.expire_button_link = Some(v);
            }
        }
        "update-always" => {
            meta.update_always = is_enabled(value);
        }
        "network-filter" => {
            if let Some(v) = non_empty(value) {
                meta.network_filter = Some(v);
            }
        }
        "subscription-refill-date" => {
            if let Some(ts) = parse_refill_date(value) {
                meta.refill_date = Some(ts);
            }
        }
        hwid if HWID_HEADERS.contains(&hwid) => {
            let status = meta.hwid.get_or_insert_with(HwidStatus::default);
            match hwid {
                "x-hwid-limit" => status.limit = parse_u32(value),
                "x-hwid-active" => status.active = parse_u32(value),
                "x-hwid-max-devices-reached" => status.max_devices_reached = header_flag(value),
                "x-hwid-not-supported" => status.not_supported = header_flag(value),
                _ => {}
            }
        }
        _ if APP_OVERRIDE_KEYS.contains(&key) => {
            if let Some(v) = non_empty(value) {
                meta.app_overrides.insert(key.to_string(), v);
            }
        }
        _ => {}
    }
}

/// Parse a `routing` value: a `happ://routing/...` link, base64 Xray routing JSON
/// (v2RayTun) or base64 Happ profile JSON.
fn parse_routing_value(value: &str) -> Option<ProviderRouting> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    if value.to_ascii_lowercase().starts_with("happ://routing/") {
        return match provider_routing::from_happ_link(value) {
            Ok(RoutingLinkAction::Add(r)) | Ok(RoutingLinkAction::AddAndActivate(r)) => Some(r),
            _ => None,
        };
    }
    let payload = match value.get(..7) {
        Some(prefix) if prefix.eq_ignore_ascii_case("base64:") => &value[7..],
        _ => value,
    };
    let text = decode_b64_str(payload).unwrap_or_else(|| payload.to_string());
    let text = text.trim().trim_start_matches('\u{feff}');
    let json: serde_json::Value = serde_json::from_str(text).ok()?;
    let obj = json.as_object()?;
    if obj.keys().any(|k| k.eq_ignore_ascii_case("rules")) {
        provider_routing::from_xray_routing(&json).ok()
    } else {
        provider_routing::from_happ_profile(&json).ok()
    }
}

/// Happ/v2RayTun enable semantics: `true`/`1` enable, any other non-empty value disables.
fn is_enabled(value: &str) -> bool {
    matches!(value.trim().to_ascii_lowercase().as_str(), "true" | "1")
}

/// A response header flag: present-but-empty counts as true; `0`/`false`/`no` are false.
fn header_flag(value: &str) -> bool {
    !matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "0" | "false" | "no" | "off"
    )
}

fn parse_u32(value: &str) -> Option<u32> {
    value.trim().parse::<u32>().ok()
}

/// `subscription-refill-date`: unix seconds, RFC 3339 or `YYYY-MM-DD`, as unix seconds.
fn parse_refill_date(value: &str) -> Option<Timestamp> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    if let Ok(n) = value.parse::<i64>() {
        return Some(n);
    }
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(value) {
        return Some(dt.timestamp());
    }
    if let Ok(date) = chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d") {
        return Some(date.and_hms_opt(0, 0, 0)?.and_utc().timestamp());
    }
    None
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
        } else if let Some((key, value)) = rest.split_once(char::is_whitespace) {
            // Happ `#providerid X` style (no colon), e.g. `#providerid Abcd1234`.
            out.push((key.trim().to_string(), value.trim().to_string()));
        }
    }
    out
}

/// Find and remove a raw `happ://routing/...` line from a (possibly base64) subscription body.
/// Returns the routing profile (first line wins) and the body with those lines removed.
pub fn split_body_routing(body: &str) -> (Option<ProviderRouting>, String) {
    let decoded = decode_body_for_meta(body);
    if !decoded.to_ascii_lowercase().contains("happ://routing/") {
        return (None, body.to_string());
    }
    let mut routing = None;
    let mut kept: Vec<&str> = Vec::new();
    for line in decoded.lines() {
        let trimmed = line.trim();
        if trimmed.to_ascii_lowercase().starts_with("happ://routing/") {
            if routing.is_none() {
                if let Ok(RoutingLinkAction::Add(r)) | Ok(RoutingLinkAction::AddAndActivate(r)) =
                    provider_routing::from_happ_link(trimmed)
                {
                    routing = Some(r);
                }
            }
            continue;
        }
        kept.push(line);
    }
    let joined = kept.join("\n");
    if matches!(decoded, std::borrow::Cow::Owned(_)) {
        use base64::engine::general_purpose::STANDARD;
        use base64::Engine;
        return (routing, STANDARD.encode(joined));
    }
    (routing, joined)
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

    #[test]
    fn fallback_url_is_backup_alias() {
        let body = "#fallback-url: https://backup.example/sub\nvless://x@h:443#A\n";
        let meta = parse_meta(&[], body);
        assert_eq!(meta.backup_urls, vec!["https://backup.example/sub"]);
    }

    #[test]
    fn providerid_header_body_and_precedence() {
        let body = "#providerid BodyId\nvless://x@h:443#A\n";
        let meta = parse_meta(&[], body);
        assert_eq!(meta.provider_id.as_deref(), Some("BodyId"));

        let headers = vec![header("providerid", "HeaderId")];
        let meta = parse_meta(&headers, body);
        assert_eq!(meta.provider_id.as_deref(), Some("HeaderId"));
    }

    #[test]
    fn routing_header_happ_link_and_xray_json() {
        use crate::model::RuleAction;
        use base64::engine::general_purpose::STANDARD;
        use base64::Engine;

        // v2RayTun: base64 of an Xray routing JSON.
        let xray = r#"{"domainStrategy":"IPIfNonMatch","rules":[{"domain":["geosite:category-ads"],"outboundTag":"block"},{"domain":["geosite:google"],"outboundTag":"proxy"}]}"#;
        let headers = vec![header(
            "routing",
            &format!("base64:{}", STANDARD.encode(xray)),
        )];
        let meta = parse_meta(&headers, "");
        let routing = meta.routing.expect("routing");
        assert_eq!(routing.domain_strategy.as_deref(), Some("IPIfNonMatch"));
        assert_eq!(routing.rules.len(), 2);
        assert_eq!(routing.rules[0].action, RuleAction::Block);

        // Happ: a `happ://routing/onadd/<b64>` link in the header.
        let profile = r#"{"Name":"P","ProxySites":["geosite:google"]}"#;
        let link = format!("happ://routing/onadd/{}", STANDARD.encode(profile));
        let headers = vec![header("routing", &link)];
        let meta = parse_meta(&headers, "");
        let routing = meta.routing.expect("routing");
        assert_eq!(routing.name, "P");
        assert_eq!(routing.rules[0].action, RuleAction::Proxy);

        // `happ://routing/off` disables routing.
        let headers = vec![header("routing", "happ://routing/off")];
        let meta = parse_meta(&headers, "");
        assert!(meta.routing.is_none());
        assert_eq!(meta.routing_enable, Some(false));
    }

    #[test]
    fn split_body_routing_removes_line() {
        use base64::engine::general_purpose::STANDARD;
        use base64::Engine;

        let profile = r#"{"Name":"Body","DirectSites":["geosite:cn"]}"#;
        let link = format!("happ://routing/add/{}", STANDARD.encode(profile));
        let body = format!("{link}\nvless://x@h:443#A\n");
        let (routing, cleaned) = split_body_routing(&body);
        assert_eq!(routing.unwrap().name, "Body");
        assert!(!cleaned.contains("happ://routing/"));
        assert!(cleaned.contains("vless://x@h:443#A"));

        // Base64 body stays base64 and keeps the link list.
        let b64 = STANDARD.encode(&body);
        let (routing, cleaned) = split_body_routing(&b64);
        assert_eq!(routing.unwrap().name, "Body");
        let decoded = STANDARD.decode(&cleaned).unwrap();
        assert!(String::from_utf8(decoded).unwrap().contains("vless://"));

        // No routing line: body unchanged.
        let (routing, same) = split_body_routing("vless://x@h:443#A\n");
        assert!(routing.is_none());
        assert_eq!(same, "vless://x@h:443#A\n");
    }

    #[test]
    fn announcement_and_management_keys() {
        let headers = vec![
            header("announce-url", "https://t.me/channel"),
            header("sub-info-text", "Maintenance tonight"),
            header("sub-info-color", "red"),
            header("sub-info-button-text", "More"),
            header("sub-info-button-link", "https://example.com/info"),
            header("sub-expire", "true"),
            header("sub-expire-button-link", "https://example.com/renew"),
            header("update-always", "true"),
            header("network-filter", "true"),
            header("subscription-refill-date", "2026-01-01"),
            header("tun-type", "singbox"),
            header("socks-auth-mode", "manual"),
            header("new-domain", "new.example.com"),
        ];
        let meta = parse_meta(&headers, "");
        assert_eq!(meta.announce_url.as_deref(), Some("https://t.me/channel"));
        let banner = meta.info_banner.unwrap();
        assert_eq!(banner.text, "Maintenance tonight");
        assert_eq!(banner.color.as_deref(), Some("red"));
        assert_eq!(banner.button_text.as_deref(), Some("More"));
        assert_eq!(
            banner.button_link.as_deref(),
            Some("https://example.com/info")
        );
        assert_eq!(meta.expire_notice.as_deref(), Some("true"));
        assert_eq!(
            meta.expire_button_link.as_deref(),
            Some("https://example.com/renew")
        );
        assert!(meta.update_always);
        assert_eq!(meta.network_filter.as_deref(), Some("true"));
        assert_eq!(meta.refill_date, Some(1_767_225_600));
        assert_eq!(
            meta.app_overrides.get("tun-type").map(String::as_str),
            Some("singbox")
        );
        assert_eq!(
            meta.app_overrides
                .get("socks-auth-mode")
                .map(String::as_str),
            Some("manual")
        );
        assert_eq!(
            meta.app_overrides.get("new-domain").map(String::as_str),
            Some("new.example.com")
        );
    }

    #[test]
    fn sub_info_text_zero_disables() {
        let headers = vec![
            header("sub-info-text", "0"),
            header("sub-info-color", "green"),
        ];
        let meta = parse_meta(&headers, "");
        let banner = meta.info_banner.unwrap();
        assert!(banner.text.is_empty());
        assert_eq!(banner.color.as_deref(), Some("green"));
    }

    #[test]
    fn hwid_headers_and_routing_enable() {
        let headers = vec![
            header("x-hwid-limit", "3"),
            header("x-hwid-active", "3"),
            header("x-hwid-max-devices-reached", "true"),
            header("x-hwid-not-supported", "0"),
        ];
        let meta = parse_meta(&headers, "");
        let hwid = meta.hwid.unwrap();
        assert_eq!(hwid.limit, Some(3));
        assert_eq!(hwid.active, Some(3));
        assert!(hwid.max_devices_reached);
        assert!(!hwid.not_supported);

        let headers = vec![header("routing-enable", "1"), header("providerid", "x")];
        assert_eq!(parse_meta(&headers, "").routing_enable, Some(true));
        let headers = vec![header("routing-enable", "0")];
        assert_eq!(parse_meta(&headers, "").routing_enable, Some(false));
    }

    #[test]
    fn refill_date_formats() {
        assert_eq!(parse_refill_date("1798761600"), Some(1_798_761_600));
        assert_eq!(parse_refill_date("2027-01-01"), Some(1_798_761_600));
        assert_eq!(
            parse_refill_date("2027-01-01T00:00:00Z"),
            Some(1_798_761_600)
        );
        assert!(parse_refill_date("not a date").is_none());
    }
}

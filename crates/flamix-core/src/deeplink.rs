//! Import deep links. OWNER: task T02.
//!
//! - `flamix://import/<url>` / `flamix://add/<url>`  (url may be percent-encoded)
//! - `happ://add/<url>`, `v2raytun://import/<url>`, `v2rayng://install-sub?url=<url>&name=..`,
//!   `clash://install-config?url=<url>`, `sing-box://import-remote-profile?url=<url>#name`
//! - a bare `https://` subscription URL or a share link.

use crate::subscription::util::decode_b64_str;
use percent_encoding::percent_decode_str;

/// What an import string refers to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Import {
    /// Subscription URL plus optional name.
    Subscription { url: String, name: Option<String> },
    /// One or more share links.
    Links(String),
}

/// Classify a deep link / pasted text. `None` if it is neither.
pub fn parse_import(text: &str) -> Option<Import> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }

    // Multi-line text where every non-empty line is a share link.
    let lines: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    if lines.len() > 1 && lines.iter().all(|line| is_share_link(line)) {
        return Some(Import::Links(text.to_string()));
    }

    for scheme in ["flamix", "happ", "v2raytun"] {
        if let Some(rest) = strip_scheme(text, scheme) {
            return parse_action_link(rest);
        }
    }
    if let Some(rest) = strip_scheme(text, "v2rayng") {
        return parse_v2rayng(rest);
    }
    if let Some(rest) = strip_scheme(text, "clash") {
        return parse_query_link(rest);
    }
    if let Some(rest) = strip_scheme(text, "sing-box") {
        return parse_singbox(rest);
    }

    if is_share_link(text) {
        return Some(Import::Links(text.to_string()));
    }
    if is_http_url(text) {
        return Some(Import::Subscription {
            url: text.to_string(),
            name: None,
        });
    }
    None
}

/// `<action>/<payload>` for `flamix://`, `happ://`, `v2raytun://`. The payload may be
/// percent-encoded or (Happ) base64-encoded.
fn parse_action_link(rest: &str) -> Option<Import> {
    let (action, payload) = rest.split_once('/')?;
    if !action.eq_ignore_ascii_case("import") && !action.eq_ignore_ascii_case("add") {
        return None;
    }
    let payload = percent_decode(payload);
    let payload = if is_http_url(&payload) || is_share_link(&payload) {
        payload
    } else {
        decode_b64_str(&payload).unwrap_or(payload)
    };
    classify_payload(&payload)
}

/// `v2rayng://install-sub?url=..&name=..`
fn parse_v2rayng(rest: &str) -> Option<Import> {
    let params = query_params(rest);
    let url = find_param(&params, "url")?;
    let name = find_param(&params, "name").filter(|n| !n.is_empty());
    match classify_url(&url) {
        Some(Import::Subscription { url, .. }) => Some(Import::Subscription { url, name }),
        other => other,
    }
}

/// `clash://install-config?url=..`
fn parse_query_link(rest: &str) -> Option<Import> {
    let params = query_params(rest);
    let url = find_param(&params, "url")?;
    classify_url(&url)
}

/// `sing-box://import-remote-profile?url=..#name`
fn parse_singbox(rest: &str) -> Option<Import> {
    let (before_fragment, fragment) = match rest.split_once('#') {
        Some((head, fragment)) => (head, Some(fragment)),
        None => (rest, None),
    };
    let params = query_params(before_fragment);
    let url = find_param(&params, "url")?;
    let name = fragment.map(percent_decode).filter(|name| !name.is_empty());
    match classify_url(&url) {
        Some(Import::Subscription { url, .. }) => Some(Import::Subscription { url, name }),
        other => other,
    }
}

fn classify_payload(text: &str) -> Option<Import> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    let lines: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    if lines.len() > 1 && lines.iter().all(|line| is_share_link(line)) {
        return Some(Import::Links(text.to_string()));
    }
    classify_url(text)
}

fn classify_url(text: &str) -> Option<Import> {
    if is_http_url(text) {
        return Some(Import::Subscription {
            url: text.to_string(),
            name: None,
        });
    }
    if is_share_link(text) {
        return Some(Import::Links(text.to_string()));
    }
    None
}

fn query_params(text: &str) -> Vec<(String, String)> {
    let query = text.split_once('?').map(|(_, q)| q).unwrap_or(text);
    url::form_urlencoded::parse(query.as_bytes())
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect()
}

fn find_param(params: &[(String, String)], key: &str) -> Option<String> {
    params
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(key))
        .map(|(_, v)| v.clone())
        .filter(|v| !v.is_empty())
}

fn strip_scheme<'a>(text: &'a str, scheme: &str) -> Option<&'a str> {
    let prefix = format!("{scheme}://");
    match text.get(..prefix.len()) {
        Some(head) if head.eq_ignore_ascii_case(&prefix) => Some(&text[prefix.len()..]),
        _ => None,
    }
}

fn percent_decode(text: &str) -> String {
    percent_decode_str(text).decode_utf8_lossy().into_owned()
}

fn scheme_of(text: &str) -> Option<&str> {
    text.split_once("://").map(|(scheme, _)| scheme)
}

fn is_http_url(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.starts_with("http://") || lower.starts_with("https://")
}

fn is_share_link(text: &str) -> bool {
    let Some(scheme) = scheme_of(text) else {
        return false;
    };
    matches!(
        scheme.to_ascii_lowercase().as_str(),
        "vless"
            | "vmess"
            | "trojan"
            | "ss"
            | "hysteria2"
            | "hy2"
            | "tuic"
            | "wireguard"
            | "wg"
            | "socks"
            | "socks5"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b64(text: &str) -> String {
        use base64::engine::general_purpose::URL_SAFE_NO_PAD;
        use base64::Engine;
        URL_SAFE_NO_PAD.encode(text)
    }

    #[test]
    fn bare_subscription_url() {
        assert_eq!(
            parse_import("https://example.com/sub?token=1"),
            Some(Import::Subscription {
                url: "https://example.com/sub?token=1".to_string(),
                name: None,
            })
        );
    }

    #[test]
    fn flamix_import_and_add() {
        assert_eq!(
            parse_import("flamix://import/https://example.com/sub"),
            Some(Import::Subscription {
                url: "https://example.com/sub".to_string(),
                name: None,
            })
        );
        assert_eq!(
            parse_import("flamix://add/https%3A%2F%2Fexample.com%2Fsub"),
            Some(Import::Subscription {
                url: "https://example.com/sub".to_string(),
                name: None,
            })
        );
    }

    #[test]
    fn happ_base64_payload() {
        let payload = b64("https://example.com/sub");
        assert_eq!(
            parse_import(&format!("happ://add/{payload}")),
            Some(Import::Subscription {
                url: "https://example.com/sub".to_string(),
                name: None,
            })
        );
    }

    #[test]
    fn v2raytun_import() {
        assert_eq!(
            parse_import("v2raytun://import/https://example.com/sub"),
            Some(Import::Subscription {
                url: "https://example.com/sub".to_string(),
                name: None,
            })
        );
    }

    #[test]
    fn v2rayng_install_sub() {
        assert_eq!(
            parse_import("v2rayng://install-sub?url=https%3A%2F%2Fexample.com%2Fsub&name=My%20Sub"),
            Some(Import::Subscription {
                url: "https://example.com/sub".to_string(),
                name: Some("My Sub".to_string()),
            })
        );
    }

    #[test]
    fn clash_install_config() {
        assert_eq!(
            parse_import("clash://install-config?url=https://example.com/config.yaml"),
            Some(Import::Subscription {
                url: "https://example.com/config.yaml".to_string(),
                name: None,
            })
        );
    }

    #[test]
    fn singbox_import_remote_profile() {
        assert_eq!(
            parse_import(
                "sing-box://import-remote-profile?url=https://example.com/sub#My%20Profile"
            ),
            Some(Import::Subscription {
                url: "https://example.com/sub".to_string(),
                name: Some("My Profile".to_string()),
            })
        );
    }

    #[test]
    fn every_share_scheme() {
        for scheme in [
            "vless",
            "vmess",
            "trojan",
            "ss",
            "hysteria2",
            "hy2",
            "tuic",
            "wireguard",
            "wg",
            "socks",
            "socks5",
        ] {
            let link = format!("{scheme}://payload");
            assert_eq!(
                parse_import(&link),
                Some(Import::Links(link.clone())),
                "scheme {scheme}"
            );
        }
    }

    #[test]
    fn multiline_links() {
        let text = "vless://a@h:443#A\nss://method:pw@h:8388#B";
        assert_eq!(parse_import(text), Some(Import::Links(text.to_string())));
    }

    #[test]
    fn rejects_unknown() {
        assert_eq!(parse_import("not a link at all"), None);
    }
}

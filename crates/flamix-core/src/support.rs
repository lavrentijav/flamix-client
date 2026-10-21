//! Kind of a provider's support link (`support-url` header), used to pick the button icon.

/// Where a support link leads. Decides the icon shown on the subscription card.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SupportKind {
    /// `t.me`, `telegram.me`, `telegram.dog` or `tg://` links.
    Telegram,
    /// `mailto:` links.
    Email,
    /// Anything else with an http(s) scheme (a website or a chat page).
    Website,
}

/// Classify a support URL. Host matching is case-insensitive and includes subdomains.
pub fn support_kind(url: &str) -> SupportKind {
    let url = url.trim();
    let lower = url.to_ascii_lowercase();
    if lower.starts_with("mailto:") {
        return SupportKind::Email;
    }
    if lower.starts_with("tg://") {
        return SupportKind::Telegram;
    }
    let rest = lower
        .strip_prefix("https://")
        .or_else(|| lower.strip_prefix("http://"))
        .unwrap_or(&lower);
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = host.rsplit('@').next().unwrap_or(host);
    let host = host.split(':').next().unwrap_or(host);
    let telegram = ["t.me", "telegram.me", "telegram.dog"];
    if telegram
        .iter()
        .any(|t| host == *t || host.ends_with(&format!(".{t}")))
    {
        SupportKind::Telegram
    } else {
        SupportKind::Website
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::valid_hwid;

    #[test]
    fn classifies_support_links() {
        assert_eq!(
            support_kind("https://t.me/support_bot"),
            SupportKind::Telegram
        );
        assert_eq!(
            support_kind("https://TELEGRAM.ME/x?y=1"),
            SupportKind::Telegram
        );
        assert_eq!(
            support_kind("tg://resolve?domain=abc"),
            SupportKind::Telegram
        );
        assert_eq!(support_kind("http://www.t.me/abc"), SupportKind::Telegram);
        assert_eq!(support_kind("https://faketg.me/abc"), SupportKind::Website);
        assert_eq!(
            support_kind("https://example.com/help"),
            SupportKind::Website
        );
        assert_eq!(
            support_kind("https://t.me.evil.example/"),
            SupportKind::Website
        );
        assert_eq!(support_kind("mailto:help@example.com"), SupportKind::Email);
        assert_eq!(
            support_kind("https://user@t.me:443/x"),
            SupportKind::Telegram
        );
    }

    #[test]
    fn hwid_rules() {
        assert!(valid_hwid("abcd1234"));
        assert!(valid_hwid("my-device_01"));
        assert!(!valid_hwid("short"));
        assert!(!valid_hwid("has space here"));
        assert!(!valid_hwid(&"a".repeat(65)));
    }
}

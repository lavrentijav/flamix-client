//! Import deep links. OWNER: task T02.
//!
//! - `flamix://import/<url>` / `flamix://add/<url>`  (url may be percent-encoded)
//! - `happ://add/<url>`, `v2raytun://import/<url>`, `v2rayng://install-sub?url=<url>&name=..`,
//!   `clash://install-config?url=<url>`, `sing-box://import-remote-profile?url=<url>#name`
//! - a bare `https://` subscription URL or a share link.

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
    let _ = text;
    todo!("T02")
}

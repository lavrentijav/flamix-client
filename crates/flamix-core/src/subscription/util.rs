//! Small shared helpers for the subscription parsers (base64 with any alphabet/padding).

use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD, URL_SAFE, URL_SAFE_NO_PAD};
use base64::Engine;

/// Decode base64 trying the standard and URL-safe alphabet, with or without padding.
/// Whitespace and newlines are ignored (subscription bodies are often wrapped).
pub fn decode_b64(s: &str) -> Option<Vec<u8>> {
    let cleaned: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    if cleaned.is_empty() {
        return None;
    }
    for engine in [STANDARD, STANDARD_NO_PAD, URL_SAFE, URL_SAFE_NO_PAD] {
        if let Ok(bytes) = engine.decode(&cleaned) {
            return Some(bytes);
        }
    }
    None
}

/// Like [`decode_b64`] but only succeeds when the result is valid UTF-8.
pub fn decode_b64_str(s: &str) -> Option<String> {
    String::from_utf8(decode_b64(s)?).ok()
}

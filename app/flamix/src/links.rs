//! Opening external links (support / site / banner buttons) in a safe way.
//!
//! Provider data is untrusted: only `http://`, `https://`, `tg://` and `mailto:` are ever handed
//! to the OS opener, and the URL is passed as a single process argument (never through a shell).

/// Longest URL we are willing to hand to the OS.
const MAX_URL_LEN: usize = 2048;

/// Normalised URL when its scheme is allowed and the text is a plausible single-line URL.
pub fn safe_url(raw: &str) -> Option<String> {
    let url = raw.trim();
    if url.is_empty() || url.len() > MAX_URL_LEN {
        return None;
    }
    if url.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return None;
    }
    let lower = url.to_ascii_lowercase();
    let ok = if let Some(rest) = lower
        .strip_prefix("https://")
        .or_else(|| lower.strip_prefix("http://"))
    {
        // A host must follow the scheme.
        rest.split(['/', '?', '#'])
            .next()
            .is_some_and(|host| !host.is_empty() && !host.starts_with('@'))
    } else if let Some(rest) = lower.strip_prefix("tg://") {
        !rest.is_empty()
    } else if let Some(rest) = lower.strip_prefix("mailto:") {
        !rest.is_empty()
    } else {
        false
    };
    ok.then(|| url.to_string())
}

/// Hand `url` (already checked with [`safe_url`]) to the operating system.
pub fn open_in_os(url: &str) -> Result<(), String> {
    use std::process::{Command, Stdio};
    #[cfg(target_os = "linux")]
    let mut cmd = {
        let mut c = Command::new("xdg-open");
        c.arg(url);
        c
    };
    #[cfg(target_os = "macos")]
    let mut cmd = {
        let mut c = Command::new("open");
        c.arg(url);
        c
    };
    // `cmd /C start` would re-parse `&`, `^`, `%` in the URL; `FileProtocolHandler` takes the URL
    // as a plain argument and hands it to ShellExecute, which is the same effect without a shell.
    #[cfg(windows)]
    let mut cmd = {
        let mut c = Command::new("rundll32");
        c.arg("url.dll,FileProtocolHandler").arg(url);
        c
    };
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    let mut cmd = Command::new("false");

    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| e.to_string())?;
    // Reap the opener in the background so it never lingers as a zombie.
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allows_only_the_four_schemes() {
        assert!(safe_url("https://t.me/support").is_some());
        assert!(safe_url("HTTP://Example.com/a?b=1&c=2").is_some());
        assert!(safe_url("tg://resolve?domain=abc").is_some());
        assert!(safe_url("mailto:help@example.com").is_some());
        assert!(safe_url("  https://example.com/x  ").as_deref() == Some("https://example.com/x"));
    }

    #[test]
    fn rejects_other_schemes_and_junk() {
        for bad in [
            "",
            "javascript:alert(1)",
            "file:///etc/passwd",
            "ftp://example.com",
            "data:text/html,hi",
            "vless://uuid@host:443",
            "example.com",
            "https://",
            "https:///path",
            "tg://",
            "mailto:",
            "https://exa mple.com",
            "https://example.com/\nx",
            "-oProxyCommand=evil",
        ] {
            assert_eq!(safe_url(bad), None, "{bad:?}");
        }
        let long = format!("https://example.com/{}", "a".repeat(MAX_URL_LEN));
        assert_eq!(safe_url(&long), None);
    }
}

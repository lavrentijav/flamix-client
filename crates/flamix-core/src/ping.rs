//! Latency tests. OWNER: task T04.

use std::time::Duration;

/// TCP connect time to `host:port` (DNS resolution excluded), `None` on failure/timeout.
pub async fn tcp_ping(host: &str, port: u16, timeout: Duration) -> Option<u32> {
    let _ = (host, port, timeout);
    todo!("T04")
}

/// HTTP GET `url` through the local proxy `proxy` (e.g. "socks5h://127.0.0.1:10808"),
/// returns time to response headers in ms (one warm-up request is not counted).
pub async fn url_ping(url: &str, proxy: &str, timeout: Duration) -> Option<u32> {
    let _ = (url, proxy, timeout);
    todo!("T04")
}

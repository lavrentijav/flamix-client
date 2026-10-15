//! Latency tests. OWNER: task T04.

use std::time::Duration;

/// TCP connect time to `host:port` (DNS resolution excluded), `None` on failure/timeout.
pub async fn tcp_ping(host: &str, port: u16, timeout: Duration) -> Option<u32> {
    let addr = tokio::net::lookup_host((host, port)).await.ok()?.next()?;
    let start = std::time::Instant::now();
    match tokio::time::timeout(timeout, tokio::net::TcpStream::connect(addr)).await {
        Ok(Ok(_stream)) => Some(elapsed_ms(start)),
        _ => None,
    }
}

/// HTTP GET `url` through the local proxy `proxy` (e.g. "socks5h://127.0.0.1:10808"),
/// returns time to response headers in ms (one warm-up request is not counted).
pub async fn url_ping(url: &str, proxy: &str, timeout: Duration) -> Option<u32> {
    let proxy = reqwest::Proxy::all(proxy).ok()?;
    let client = reqwest::Client::builder()
        .proxy(proxy)
        .redirect(reqwest::redirect::Policy::none())
        .timeout(timeout)
        .build()
        .ok()?;

    // Warm-up request, deliberately not counted (opens DNS/connection caches).
    let _ = client.get(url).send().await;

    let start = std::time::Instant::now();
    match client.get(url).send().await {
        Ok(response) if response.status().is_success() || response.status().is_redirection() => {
            Some(elapsed_ms(start))
        }
        _ => None,
    }
}

/// TCP-ping several targets with a bounded number of concurrent connections.
/// Each target is `(id, host, port)`; the result keeps the input order and maps every id to
/// its latency (`None` = failed/timed out).
pub async fn tcp_ping_many(
    targets: Vec<(String, String, u16)>,
    timeout: Duration,
    concurrency: usize,
) -> Vec<(String, Option<u32>)> {
    use std::collections::HashMap;
    use std::sync::Arc;
    use tokio::sync::Semaphore;
    use tokio::task::JoinSet;

    let permits = Arc::new(Semaphore::new(concurrency.max(1)));
    let mut order = Vec::with_capacity(targets.len());
    let mut tasks = JoinSet::new();

    for (id, host, port) in targets {
        order.push(id.clone());
        let permits = Arc::clone(&permits);
        tasks.spawn(async move {
            let _permit = permits.acquire_owned().await;
            (id, tcp_ping(&host, port, timeout).await)
        });
    }

    let mut results: HashMap<String, Option<u32>> = HashMap::new();
    while let Some(joined) = tasks.join_next().await {
        if let Ok((id, ms)) = joined {
            results.insert(id, ms);
        }
    }

    order
        .into_iter()
        .map(|id| (id.clone(), results.remove(&id).flatten()))
        .collect()
}

/// Milliseconds rounded up, with a floor of 1 ms.
fn elapsed_ms(start: std::time::Instant) -> u32 {
    let micros = start.elapsed().as_micros();
    let ms = micros.div_ceil(1000).max(1);
    ms.min(u32::MAX as u128) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn closed_local_port() -> u16 {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        port
    }

    #[tokio::test]
    async fn tcp_ping_reachable() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();

        let ms = tcp_ping("127.0.0.1", port, Duration::from_secs(2)).await;
        assert!(ms.is_some());
        assert!(ms.unwrap() >= 1);
    }

    #[tokio::test]
    async fn tcp_ping_closed_port() {
        let port = closed_local_port().await;
        assert_eq!(
            tcp_ping("127.0.0.1", port, Duration::from_millis(500)).await,
            None
        );
    }

    #[tokio::test]
    async fn tcp_ping_unroutable_times_out() {
        let ms = tcp_ping("10.255.255.1", 443, Duration::from_millis(300)).await;
        assert_eq!(ms, None);
    }

    #[tokio::test]
    async fn tcp_ping_many_keeps_order() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let open = listener.local_addr().unwrap().port();
        let closed = closed_local_port().await;

        let targets = vec![
            ("a".to_string(), "127.0.0.1".to_string(), closed),
            ("b".to_string(), "127.0.0.1".to_string(), open),
        ];
        let results = tcp_ping_many(targets, Duration::from_secs(2), 4).await;

        assert_eq!(results.len(), 2);
        assert_eq!(results[0].0, "a");
        assert_eq!(results[1].0, "b");
        assert_eq!(results[0].1, None);
        assert!(results[1].1.is_some());
    }

    #[tokio::test]
    async fn url_ping_dead_proxy_returns_none() {
        let port = closed_local_port().await;
        let proxy = format!("socks5h://127.0.0.1:{port}");

        let start = std::time::Instant::now();
        let ms = url_ping(
            "http://example.invalid/generate_204",
            &proxy,
            Duration::from_millis(800),
        )
        .await;

        assert_eq!(ms, None);
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "a dead local proxy should fail fast"
        );
    }
}

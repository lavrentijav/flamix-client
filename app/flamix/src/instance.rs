//! Single-instance coordination.
//!
//! The first Flamix process listens on a random localhost port and writes the port plus a
//! random token to `<data_dir>/instance`. A later process reads that file, forwards its
//! arguments and exits. The running instance verifies the token and processes the arguments
//! like an import, then brings its window to the front. A stale file (connection refused)
//! makes the new process take over as the first instance.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::io::AsyncReadExt;
use tokio::net::TcpListener;

/// Contents of the `<data_dir>/instance` file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstanceInfo {
    pub port: u16,
    pub token: String,
}

/// Path of the instance file.
pub fn instance_path(dir: &Path) -> PathBuf {
    dir.join("instance")
}

/// Serialise the instance file (`port` line, `token` line).
pub fn encode(info: &InstanceInfo) -> String {
    format!("{}\n{}\n", info.port, info.token)
}

/// Parse the instance file; `None` when malformed.
pub fn decode(text: &str) -> Option<InstanceInfo> {
    let mut lines = text.lines();
    let port = lines.next()?.trim().parse::<u16>().ok()?;
    let token = lines.next()?.trim().to_string();
    if token.is_empty() {
        return None;
    }
    Some(InstanceInfo { port, token })
}

/// Message a second process sends: `token` line, JSON-encoded args line.
pub fn encode_message(token: &str, args: &[String]) -> String {
    let json = serde_json::to_string(args).unwrap_or_else(|_| "[]".to_string());
    format!("{token}\n{json}\n")
}

/// Decode a forwarded message, verifying the token. `None` on mismatch or malformed input.
pub fn decode_message(token: &str, text: &str) -> Option<Vec<String>> {
    let mut lines = text.lines();
    if lines.next()?.trim() != token {
        return None;
    }
    let json = lines.next()?.trim();
    serde_json::from_str(json).ok()
}

/// Forward `args` to the running instance. Blocking; used before the runtime starts.
pub fn forward(info: &InstanceInfo, args: &[String]) -> std::io::Result<()> {
    use std::io::Write;
    let addr = SocketAddr::from(([127, 0, 0, 1], info.port));
    let mut stream = std::net::TcpStream::connect_timeout(&addr, std::time::Duration::from_secs(2))?;
    stream.write_all(encode_message(&info.token, args).as_bytes())?;
    stream.flush()?;
    Ok(())
}

/// Accept forwarded argument lists forever. `handler` is invoked with the decoded args.
pub async fn serve<H>(listener: TcpListener, token: String, handler: H)
where
    H: Fn(Vec<String>) + Send + Sync + 'static,
{
    let handler = Arc::new(handler);
    loop {
        let Ok((mut stream, _)) = listener.accept().await else {
            continue;
        };
        let token = token.clone();
        let handler = Arc::clone(&handler);
        tokio::spawn(async move {
            let mut buf = Vec::new();
            if stream.read_to_end(&mut buf).await.is_err() {
                return;
            }
            let text = String::from_utf8_lossy(&buf);
            if let Some(args) = decode_message(&token, &text) {
                handler(args);
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn info_roundtrip() {
        let info = InstanceInfo {
            port: 43210,
            token: "deadbeef".to_string(),
        };
        let text = encode(&info);
        assert_eq!(decode(&text), Some(info));
    }

    #[test]
    fn decode_rejects_garbage() {
        assert_eq!(decode(""), None);
        assert_eq!(decode("not-a-port\nabc"), None);
        assert_eq!(decode("1234\n"), None);
    }

    #[test]
    fn message_roundtrip_and_token_check() {
        let args = vec![
            "flamix://import/https://example.com/sub".to_string(),
            "--minimized".to_string(),
        ];
        let message = encode_message("secret", &args);
        assert_eq!(decode_message("secret", &message), Some(args.clone()));
        assert_eq!(decode_message("wrong", &message), None);
        assert_eq!(decode_message("secret", "garbage"), None);
        assert_eq!(decode_message("secret", "secret\nnot-json"), None);
    }
}

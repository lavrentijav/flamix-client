//! Traffic counters of the running cores. OWNER: task T05.
//!
//! Communication is deliberately tiny: Xray through its `xray api statsquery` CLI and sing-box
//! through a raw HTTP/1.1 request to the clash API, so no extra HTTP client is pulled in here.

use std::path::Path;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

/// Query Xray for the proxy outbound traffic since the previous call.
///
/// Runs `xray api statsquery --server=127.0.0.1:<port> -pattern "outbound>>>proxy>>>traffic"
/// -reset`, which resets the counters after reading them, so the returned pair already is the
/// delta since the last call. Returns `(uplink, downlink)` in bytes.
pub async fn xray_traffic(xray_bin: &Path, api_port: u16) -> crate::Result<(u64, u64)> {
    let server = format!("--server=127.0.0.1:{api_port}");
    let output = tokio::process::Command::new(xray_bin)
        .args([
            "api",
            "statsquery",
            &server,
            "-pattern",
            "outbound>>>proxy>>>traffic",
            "-reset",
        ])
        .output()
        .await?;
    if !output.status.success() {
        return Err(crate::Error::Command(format!(
            "xray api statsquery: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    parse_xray_stats(&String::from_utf8_lossy(&output.stdout))
}

/// Parse the JSON printed by `xray api statsquery`, summing uplink and downlink values.
///
/// Sample:
/// `{"stat": [{"name": "outbound>>>proxy>>>traffic>>>uplink", "value": "1234"},
///            {"name": "outbound>>>proxy>>>traffic>>>downlink", "value": 5678}]}`
pub fn parse_xray_stats(json: &str) -> crate::Result<(u64, u64)> {
    let value: serde_json::Value = serde_json::from_str(json)
        .map_err(|e| crate::Error::Command(format!("statsquery json: {e}")))?;
    let mut uplink = 0u64;
    let mut downlink = 0u64;
    if let Some(stats) = value.get("stat").and_then(serde_json::Value::as_array) {
        for item in stats {
            let name = item
                .get("name")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("");
            let amount = stat_value(item.get("value"));
            if name.contains("uplink") {
                uplink = uplink.saturating_add(amount);
            } else if name.contains("downlink") {
                downlink = downlink.saturating_add(amount);
            }
        }
    }
    Ok((uplink, downlink))
}

fn stat_value(value: Option<&serde_json::Value>) -> u64 {
    match value {
        Some(serde_json::Value::Number(n)) => n.as_u64().unwrap_or(0),
        Some(serde_json::Value::String(s)) => s.parse().unwrap_or(0),
        _ => 0,
    }
}

/// Read the clash API `/connections` totals of sing-box. The caller computes deltas.
///
/// Returns `(uploadTotal, downloadTotal)` in bytes.
pub async fn singbox_traffic(api_port: u16) -> crate::Result<(u64, u64)> {
    let body = http_get(api_port, "/connections").await?;
    let text = String::from_utf8_lossy(&body);
    parse_clash_connections(&text)
}

/// Parse a clash API `/connections` response body (plain JSON).
pub fn parse_clash_connections(body: &str) -> crate::Result<(u64, u64)> {
    let value: serde_json::Value = serde_json::from_str(body)
        .map_err(|e| crate::Error::Command(format!("clash connections json: {e}")))?;
    let upload = value
        .get("uploadTotal")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    let download = value
        .get("downloadTotal")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    Ok((upload, download))
}

/// Minimal HTTP/1.1 GET returning the response body. Handles `Content-Length`, chunked and
/// connection-close responses.
async fn http_get(port: u16, path: &str) -> crate::Result<Vec<u8>> {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).await?;
    let request = format!(
        "GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAccept: application/json\r\nConnection: close\r\n\r\n"
    );
    stream.write_all(request.as_bytes()).await?;
    let mut response = Vec::new();
    stream.read_to_end(&mut response).await?;

    let header_end = find_subsequence(&response, b"\r\n\r\n")
        .ok_or_else(|| crate::Error::Command("malformed HTTP response".to_string()))?;
    let head = String::from_utf8_lossy(&response[..header_end]).to_ascii_lowercase();
    let body = &response[header_end + 4..];
    if head.contains("transfer-encoding: chunked") {
        dechunk(body)
    } else {
        Ok(body.to_vec())
    }
}

fn find_subsequence(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn dechunk(body: &[u8]) -> crate::Result<Vec<u8>> {
    let mut out = Vec::new();
    let mut pos = 0usize;
    loop {
        let line_end = find_subsequence(&body[pos..], b"\r\n")
            .ok_or_else(|| crate::Error::Command("malformed chunked body".to_string()))?;
        let size_line = std::str::from_utf8(&body[pos..pos + line_end])
            .map_err(|e| crate::Error::Command(format!("chunk size: {e}")))?;
        let size_text = size_line.split(';').next().unwrap_or("").trim();
        let size = usize::from_str_radix(size_text, 16)
            .map_err(|e| crate::Error::Command(format!("chunk size: {e}")))?;
        pos += line_end + 2;
        if size == 0 {
            break;
        }
        if pos + size > body.len() {
            return Err(crate::Error::Command("truncated chunk".to_string()));
        }
        out.extend_from_slice(&body[pos..pos + size]);
        pos += size + 2;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_statsquery_string_and_number_values() {
        let sample = r#"{"stat":[
            {"name":"outbound>>>proxy>>>traffic>>>uplink","value":"1234"},
            {"name":"outbound>>>proxy>>>traffic>>>downlink","value":5678}
        ]}"#;
        assert_eq!(parse_xray_stats(sample).unwrap(), (1234, 5678));
    }

    #[test]
    fn parses_multiple_matching_stats() {
        let sample = r#"{"stat":[
            {"name":"outbound>>>proxy>>>traffic>>>uplink","value":"10"},
            {"name":"outbound>>>proxy>>>traffic>>>uplink","value":5},
            {"name":"outbound>>>proxy>>>traffic>>>downlink","value":"7"}
        ]}"#;
        assert_eq!(parse_xray_stats(sample).unwrap(), (15, 7));
    }

    #[test]
    fn rejects_bad_stats_json() {
        assert!(parse_xray_stats("not json").is_err());
    }

    #[test]
    fn parses_clash_connections() {
        let sample = r#"{"downloadTotal":2048,"uploadTotal":1024,"connections":[]}"#;
        assert_eq!(parse_clash_connections(sample).unwrap(), (1024, 2048));
    }

    #[test]
    fn dechunks_body() {
        let chunked = b"4\r\nWiki\r\n5\r\npedia\r\n0\r\n\r\n";
        assert_eq!(dechunk(chunked).unwrap(), b"Wikipedia");
    }
}

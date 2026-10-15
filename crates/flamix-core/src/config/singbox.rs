//! sing-box JSON config (1.12+ format). OWNER: task T03.

use crate::model::{Server, Settings};

/// Outbound object (`"tag": "proxy"`) for `server`; `None` if sing-box can't express it.
pub fn outbound(server: &Server, settings: &Settings) -> Option<serde_json::Value> {
    let _ = (server, settings);
    todo!("T03")
}

/// Full config. `tun` = add a tun inbound (auto_route, strict_route) instead of mixed.
/// `upstream_socks` = Some(port): the proxy outbound is SOCKS to 127.0.0.1:port (Xray chain).
/// `api_port`: `experimental.clash_api` on 127.0.0.1 for traffic stats.
pub fn full_config(
    server: &Server,
    settings: &Settings,
    tun: bool,
    upstream_socks: Option<u16>,
    api_port: u16,
) -> serde_json::Value {
    let _ = (server, settings, tun, upstream_socks, api_port);
    todo!("T03")
}

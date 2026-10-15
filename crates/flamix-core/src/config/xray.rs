//! Xray-core JSON config. OWNER: task T03.

use crate::model::{Server, Settings};

/// Outbound object (`"tag": "proxy"`) for `server`.
pub fn outbound(server: &Server, settings: &Settings) -> serde_json::Value {
    let _ = (server, settings);
    todo!("T03")
}

/// Full config: log, dns, inbounds (socks + http on `settings` ports, plus `api` inbound
/// for stats on 127.0.0.1:`api_port`), outbounds (proxy, direct, block), routing from
/// `settings.routing`, `stats` + `policy` for traffic counters.
pub fn full_config(server: &Server, settings: &Settings, api_port: u16) -> serde_json::Value {
    let _ = (server, settings, api_port);
    todo!("T03")
}

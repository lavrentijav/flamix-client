//! Proxy core configuration generators. OWNER: task T03.
//!
//! Flamix drives two external cores, like v2rayN:
//! - **Xray-core** for Proxy mode and for every protocol it supports (vless incl. reality/xhttp,
//!   vmess, trojan, shadowsocks, socks, http, wireguard);
//! - **sing-box** for hysteria2 / tuic (Proxy mode) and for TUN mode: its `tun` inbound
//!   captures traffic and forwards it to the selected protocol's outbound directly (sing-box
//!   protocols) or to Xray's local SOCKS inbound (Xray-only features like xhttp).

pub mod singbox;
pub mod xray;

use crate::model::{ConnectionMode, Protocol, Server, Settings};

/// Which external core binary to run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoreKind {
    Xray,
    SingBox,
}

impl CoreKind {
    /// Executable base name (without `.exe`).
    pub fn binary_name(self) -> &'static str {
        match self {
            Self::Xray => "xray",
            Self::SingBox => "sing-box",
        }
    }
}

/// One process to start: which core and the JSON config to feed it.
#[derive(Debug, Clone, PartialEq)]
pub struct CorePlan {
    pub core: CoreKind,
    pub config: serde_json::Value,
    /// Needs admin/root (TUN).
    pub elevated: bool,
    /// Stats endpoint: Xray gRPC API port (`xray api statsquery`) or sing-box clash_api port.
    pub api_port: u16,
}

/// Port of the stats API for a core: Xray = `socks_port + 10`, sing-box = `socks_port + 11`.
pub fn api_port(kind: CoreKind, settings: &Settings) -> u16 {
    let off = match kind {
        CoreKind::Xray => 10,
        CoreKind::SingBox => 11,
    };
    settings.socks_port.saturating_add(off)
}

/// Cores that can run `server` as an outbound.
pub fn supported_by(server: &Server) -> &'static [CoreKind] {
    match server.protocol() {
        Protocol::Hysteria2 | Protocol::Tuic => &[CoreKind::SingBox],
        _ => &[CoreKind::Xray, CoreKind::SingBox],
    }
}

/// Plan the processes for connecting to `server` with `settings`.
///
/// - Proxy mode, Xray-capable server: one Xray process.
/// - Proxy mode, sing-box-only server: one sing-box process (mixed inbound).
/// - TUN mode: one elevated sing-box process with a `tun` inbound; if the server needs Xray
///   (xhttp transport, or anything sing-box can't express) an extra Xray process provides a
///   local SOCKS inbound that sing-box's outbound points to.
pub fn plan(server: &Server, settings: &Settings) -> Vec<CorePlan> {
    let _ = (server, settings, ConnectionMode::Proxy);
    todo!("T03")
}

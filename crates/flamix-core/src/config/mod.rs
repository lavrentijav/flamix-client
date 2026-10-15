//! Proxy core configuration generators. OWNER: task T03.
//!
//! Flamix drives two external cores, like v2rayN:
//! - **Xray-core** for Proxy mode and for every protocol it supports (vless incl. reality/xhttp,
//!   vmess, trojan, shadowsocks, socks, http, wireguard);
//! - **sing-box** for hysteria2 / tuic (Proxy mode) and for TUN mode: its `tun` inbound
//!   captures traffic and forwards it to the selected protocol's outbound directly (sing-box
//!   protocols) or to Xray's local SOCKS inbound (Xray-only features like xhttp).

mod routing;
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
    match settings.mode {
        ConnectionMode::Proxy => {
            if supported_by(server).contains(&CoreKind::Xray) {
                let api_port = api_port(CoreKind::Xray, settings);
                vec![CorePlan {
                    core: CoreKind::Xray,
                    config: xray::full_config(server, settings, api_port),
                    elevated: false,
                    api_port,
                }]
            } else {
                let api_port = api_port(CoreKind::SingBox, settings);
                vec![CorePlan {
                    core: CoreKind::SingBox,
                    config: singbox::full_config(server, settings, false, None, api_port),
                    elevated: false,
                    api_port,
                }]
            }
        }
        ConnectionMode::Tun => {
            // sing-box captures the TUN and forwards to the protocol outbound, or to a
            // local SOCKS inbound run by Xray when the transport is Xray-only (xhttp/kcp).
            let upstream_socks = singbox::outbound(server, settings)
                .is_none()
                .then_some(settings.socks_port);
            let mut plans = Vec::new();
            if upstream_socks.is_some() {
                let api_port = api_port(CoreKind::Xray, settings);
                plans.push(CorePlan {
                    core: CoreKind::Xray,
                    config: xray::full_config(server, settings, api_port),
                    elevated: false,
                    api_port,
                });
            }
            let api_port = api_port(CoreKind::SingBox, settings);
            plans.push(CorePlan {
                core: CoreKind::SingBox,
                config: singbox::full_config(server, settings, true, upstream_socks, api_port),
                elevated: true,
                api_port,
            });
            plans
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Credentials, Network, Security, Transport};

    fn server(credentials: Credentials, transport: Transport) -> Server {
        Server {
            id: "test".into(),
            name: "test".into(),
            country: None,
            address: "example.com".into(),
            port: 443,
            credentials,
            transport,
            security: Security::default(),
            raw_link: None,
            traffic: None,
            local_traffic: Default::default(),
            last_ping_ms: None,
        }
    }

    fn vless_xhttp() -> Server {
        server(
            Credentials::Vless {
                uuid: "u".into(),
                flow: None,
                encryption: None,
            },
            Transport {
                network: Network::Xhttp,
                path: Some("/x".into()),
                ..Transport::default()
            },
        )
    }

    #[test]
    fn api_ports() {
        let settings = Settings::default();
        assert_eq!(api_port(CoreKind::Xray, &settings), 10818);
        assert_eq!(api_port(CoreKind::SingBox, &settings), 10819);
    }

    #[test]
    fn proxy_hy2_is_singbox_only() {
        let s = server(
            Credentials::Hysteria2 {
                password: "p".into(),
                obfs: None,
                obfs_password: None,
                ports: None,
                up_mbps: None,
                down_mbps: None,
            },
            Transport::default(),
        );
        let plan = plan(&s, &Settings::default());
        assert_eq!(plan.len(), 1);
        assert_eq!(plan[0].core, CoreKind::SingBox);
        assert!(!plan[0].elevated);
        assert_eq!(plan[0].api_port, 10819);
        assert_eq!(plan[0].config["inbounds"][0]["type"], "mixed");
    }

    #[test]
    fn proxy_vless_uses_xray() {
        let plan = plan(&vless_xhttp(), &Settings::default());
        assert_eq!(plan.len(), 1);
        assert_eq!(plan[0].core, CoreKind::Xray);
        assert!(!plan[0].elevated);
        assert_eq!(plan[0].api_port, 10818);
        assert_eq!(plan[0].config["inbounds"][0]["tag"], "socks");
    }

    #[test]
    fn tun_xhttp_chains_xray_and_singbox() {
        let settings = Settings {
            mode: ConnectionMode::Tun,
            ..Settings::default()
        };
        let plan = plan(&vless_xhttp(), &settings);
        assert_eq!(plan.len(), 2);
        assert_eq!(plan[0].core, CoreKind::Xray);
        assert!(!plan[0].elevated);
        assert_eq!(plan[1].core, CoreKind::SingBox);
        assert!(plan[1].elevated);
        // sing-box routes through Xray's SOCKS inbound and does not bind mixed.
        assert_eq!(plan[1].config["outbounds"][0]["type"], "socks");
        assert_eq!(plan[1].config["outbounds"][0]["server_port"], 10808);
        assert_eq!(plan[1].config["inbounds"][0]["type"], "tun");
    }

    #[test]
    fn tun_regular_protocol_singbox_only() {
        let settings = Settings {
            mode: ConnectionMode::Tun,
            ..Settings::default()
        };
        let s = server(
            Credentials::Vless {
                uuid: "u".into(),
                flow: None,
                encryption: None,
            },
            Transport {
                network: Network::Ws,
                path: Some("/ws".into()),
                ..Transport::default()
            },
        );
        let plan = plan(&s, &settings);
        assert_eq!(plan.len(), 1);
        assert_eq!(plan[0].core, CoreKind::SingBox);
        assert!(plan[0].elevated);
        assert_eq!(plan[0].config["inbounds"][0]["type"], "tun");
    }
}

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

/// Exhaustive validation of generated configs against the real core binaries.
///
/// The binaries are the oracle: `FLAMIX_XRAY_BIN` / `FLAMIX_SINGBOX_BIN`. When unset
/// the tests return early (so normal `cargo test` stays hermetic). Every protocol
/// sample x {Proxy, TUN} x {Global, BypassRu, Custom(each rule kind)} is validated,
/// plus the TUN xhttp chain. Any non-zero exit *or* deprecation warning fails with
/// the config name and the core's own message.
#[cfg(test)]
mod validation {
    use super::*;
    use crate::model::{
        Credentials, Dns, Network, Routing, RoutingPreset, RoutingRule, RuleAction, Security,
        Transport,
    };
    use std::sync::atomic::{AtomicUsize, Ordering};

    static COUNTER: AtomicUsize = AtomicUsize::new(0);

    pub(crate) fn write_temp(name: &str, content: &str) -> std::path::PathBuf {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("flamix-config-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let safe: String = name
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        let path = dir.join(format!("{n}-{safe}.json"));
        std::fs::write(&path, content).unwrap();
        path
    }

    fn server(credentials: Credentials, transport: Transport, security: Security) -> Server {
        Server {
            id: "sample".into(),
            name: "sample".into(),
            country: None,
            address: "example.com".into(),
            port: 443,
            credentials,
            transport,
            security,
            raw_link: None,
            traffic: None,
            local_traffic: Default::default(),
            last_ping_ms: None,
        }
    }

    fn vless(flow: Option<&str>) -> Credentials {
        Credentials::Vless {
            uuid: "11111111-1111-1111-1111-111111111111".into(),
            flow: flow.map(str::to_string),
            encryption: Some("none".into()),
        }
    }

    fn tls(sni: &str) -> Security {
        Security::Tls {
            sni: Some(sni.into()),
            alpn: vec!["h2".into(), "http/1.1".into()],
            fingerprint: Some("chrome".into()),
            allow_insecure: false,
        }
    }

    /// Every protocol/transport combination the generators can emit.
    fn samples() -> Vec<(&'static str, Server)> {
        let mut v = Vec::new();
        v.push((
            "vless-reality",
            server(
                vless(Some("xtls-rprx-vision")),
                Transport::default(),
                Security::Reality {
                    sni: Some("www.microsoft.com".into()),
                    // x25519 public keys are base64url (`xray x25519`), no padding.
                    public_key: "rLj6M416xXQjTr7KlO_xpBxxpUaM5u_e_SOeNhHhtVQ".into(),
                    short_id: Some("abcd".into()),
                    spider_x: Some("/".into()),
                    fingerprint: Some("chrome".into()),
                },
            ),
        ));
        v.push((
            "vless-xhttp",
            server(
                vless(None),
                Transport {
                    network: Network::Xhttp,
                    path: Some("/x".into()),
                    host: Some("h.example".into()),
                    mode: Some("packet-up".into()),
                    extra: Some(serde_json::json!({"scMaxEachPostBytes": 1000000})),
                    ..Transport::default()
                },
                tls("h.example"),
            ),
        ));
        v.push((
            "vless-ws",
            server(
                vless(None),
                Transport {
                    network: Network::Ws,
                    path: Some("/ws".into()),
                    host: Some("ws.example".into()),
                    ..Transport::default()
                },
                tls("ws.example"),
            ),
        ));
        v.push((
            "vless-grpc",
            server(
                vless(None),
                Transport {
                    network: Network::Grpc,
                    service_name: Some("gsvc".into()),
                    mode: Some("multi".into()),
                    ..Transport::default()
                },
                tls("grpc.example"),
            ),
        ));
        v.push((
            "vless-h2",
            server(
                vless(None),
                Transport {
                    network: Network::H2,
                    path: Some("/h2".into()),
                    host: Some("a.example,b.example".into()),
                    ..Transport::default()
                },
                tls("h2.example"),
            ),
        ));
        v.push((
            "vless-httpupgrade",
            server(
                vless(None),
                Transport {
                    network: Network::HttpUpgrade,
                    path: Some("/hu".into()),
                    host: Some("hu.example".into()),
                    ..Transport::default()
                },
                tls("hu.example"),
            ),
        ));
        v.push((
            "vless-tcp-http",
            server(
                vless(None),
                Transport {
                    network: Network::Tcp,
                    header_type: Some("http".into()),
                    path: Some("/tcp".into()),
                    host: Some("tcp.example".into()),
                    ..Transport::default()
                },
                Security::None,
            ),
        ));
        v.push((
            "vless-kcp",
            server(
                vless(None),
                Transport {
                    network: Network::Kcp,
                    header_type: Some("none".into()),
                    seed: Some("seed1".into()),
                    ..Transport::default()
                },
                Security::None,
            ),
        ));
        v.push((
            "vless-quic",
            server(
                vless(None),
                Transport {
                    network: Network::Quic,
                    header_type: Some("none".into()),
                    seed: Some("key1".into()),
                    ..Transport::default()
                },
                tls("quic.example"),
            ),
        ));
        v.push((
            "vmess-ws",
            server(
                Credentials::Vmess {
                    uuid: "22222222-2222-2222-2222-222222222222".into(),
                    alter_id: 0,
                    cipher: Some("auto".into()),
                },
                Transport {
                    network: Network::Ws,
                    path: Some("/vm".into()),
                    host: Some("vm.example".into()),
                    ..Transport::default()
                },
                tls("vm.example"),
            ),
        ));
        v.push((
            "trojan-grpc",
            server(
                Credentials::Trojan {
                    password: "secret".into(),
                },
                Transport {
                    network: Network::Grpc,
                    service_name: Some("gsvc".into()),
                    ..Transport::default()
                },
                tls("tr.example"),
            ),
        ));
        v.push((
            "shadowsocks",
            server(
                Credentials::Shadowsocks {
                    method: "aes-256-gcm".into(),
                    password: "pw".into(),
                    plugin: Some("obfs-local".into()),
                    plugin_opts: Some("obfs=http;obfs-host=x.example".into()),
                },
                Transport::default(),
                Security::None,
            ),
        ));
        v.push((
            "socks-auth",
            server(
                Credentials::Socks {
                    username: Some("u".into()),
                    password: Some("p".into()),
                },
                Transport::default(),
                Security::None,
            ),
        ));
        v.push((
            "http-auth",
            server(
                Credentials::Http {
                    username: Some("u".into()),
                    password: Some("p".into()),
                },
                Transport::default(),
                Security::None,
            ),
        ));
        v.push((
            "wireguard",
            server(
                Credentials::Wireguard {
                    private_key: "yAnz5TF+lXXJte14tji3zlMNq+hd2rYUIgJBgB3fBmk=".into(),
                    peer_public_key: "xTIBA5rboUvnH4htodjb6e697QjLERt1NAB4mZqp8Dg=".into(),
                    pre_shared_key: Some("yAnz5TF+lXXJte14tji3zlMNq+hd2rYUIgJBgB3fBmk=".into()),
                    local_address: vec!["10.0.0.2/32".into()],
                    reserved: Some([1, 2, 3]),
                    mtu: Some(1420),
                },
                Transport::default(),
                Security::None,
            ),
        ));
        v.push((
            "hysteria2-hopping",
            server(
                Credentials::Hysteria2 {
                    password: "pw".into(),
                    obfs: Some("salamander".into()),
                    obfs_password: Some("op".into()),
                    ports: Some("20000-30000".into()),
                    up_mbps: Some(100),
                    down_mbps: Some(200),
                },
                Transport::default(),
                tls("hy.example"),
            ),
        ));
        v.push((
            "tuic",
            server(
                Credentials::Tuic {
                    uuid: "33333333-3333-3333-3333-333333333333".into(),
                    password: "secret".into(),
                    congestion_control: Some("bbr".into()),
                    udp_relay_mode: Some("native".into()),
                },
                Transport::default(),
                tls("tuic.example"),
            ),
        ));
        v
    }

    /// A custom-rules preset that uses one rule of every kind.
    fn custom_routing() -> Routing {
        Routing {
            preset: RoutingPreset::Custom,
            custom_rules: vec![
                RoutingRule {
                    domains: vec![
                        "domain:suffix.example".into(),
                        "full:exact.example".into(),
                        "regexp:^ads[0-9]+\\.example$".into(),
                        "geosite:category-ads-all".into(),
                    ],
                    ips: vec![
                        "geoip:cn".into(),
                        "1.2.3.0/24".into(),
                        "geoip:private".into(),
                    ],
                    processes: vec!["telegram.exe".into()],
                    network: Some("tcp".into()),
                    ports: Some("443".into()),
                    action: RuleAction::Direct,
                    enabled: true,
                },
                RoutingRule {
                    domains: vec![],
                    ips: vec!["geoip:ru".into()],
                    processes: vec![],
                    network: Some("udp".into()),
                    ports: Some("1000-2000".into()),
                    action: RuleAction::Block,
                    enabled: true,
                },
                RoutingRule {
                    domains: vec!["domain:proxy.example".into()],
                    ips: vec![],
                    processes: vec![],
                    network: None,
                    ports: None,
                    action: RuleAction::Proxy,
                    enabled: true,
                },
            ],
            domain_strategy: "IPIfNonMatch".into(),
            block_quic: true,
        }
    }

    fn presets() -> Vec<(&'static str, Routing)> {
        vec![
            (
                "global",
                Routing {
                    preset: RoutingPreset::Global,
                    ..Routing::default()
                },
            ),
            (
                "bypass-ru",
                Routing {
                    preset: RoutingPreset::BypassRu,
                    ..Routing::default()
                },
            ),
            ("custom", custom_routing()),
        ]
    }

    fn settings(mode: ConnectionMode, routing: Routing) -> Settings {
        Settings {
            mode,
            routing,
            dns: Dns::default(),
            ..Settings::default()
        }
    }

    fn xray_supports(server: &Server) -> bool {
        !matches!(server.protocol(), Protocol::Hysteria2 | Protocol::Tuic)
    }

    fn singbox_supports(server: &Server, settings: &Settings) -> bool {
        singbox::outbound(server, settings).is_some()
    }

    /// Runs the core's config checker. Returns `None` when the binary is not set,
    /// otherwise `Some(problem)` with the core's own message when validation fails.
    fn check_core(core: CoreKind, cfg: &serde_json::Value, name: &str) -> Option<String> {
        let bin = match core {
            CoreKind::Xray => std::env::var("FLAMIX_XRAY_BIN"),
            CoreKind::SingBox => std::env::var("FLAMIX_SINGBOX_BIN"),
        };
        let bin = bin.ok()?;
        let file = write_temp(name, &serde_json::to_string(cfg).unwrap());
        let args: &[&str] = match core {
            CoreKind::Xray => &["run", "-test", "-c"],
            CoreKind::SingBox => &["check", "-c"],
        };
        let out = std::process::Command::new(bin)
            .args(args)
            .arg(&file)
            .output()
            .unwrap();
        let message = format!(
            "{}\n{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        let _ = std::fs::remove_file(&file);
        if !out.status.success() {
            return Some(format!(
                "{} rejected '{name}':\n{message}",
                core.binary_name()
            ));
        }
        if is_deprecation(&message) {
            return Some(format!(
                "{} deprecation warning in '{name}':\n{message}",
                core.binary_name()
            ));
        }
        None
    }

    /// True for a deprecation warning about the generated config itself.
    ///
    /// Xray v26 unconditionally advises against whole protocols/transports the user
    /// chose (Shadowsocks, VMess, Trojan, WebSocket, gRPC): "The feature X ... is
    /// deprecated, not recommended for using and might be removed." Those are
    /// recommendations for the server's protocol, not deprecated fields in our
    /// output, and no config can avoid them while those links stay supported.
    /// Field-level warnings (e.g. `headers.host`) are still failures.
    fn is_deprecation(message: &str) -> bool {
        let lower = message.to_lowercase();
        lower.contains("deprecat")
            && !lower.contains("not recommended for using and might be removed")
    }

    fn report(problems: Vec<String>) {
        if !problems.is_empty() {
            panic!(
                "{} config(s) rejected by the real cores:\n\n{}",
                problems.len(),
                problems.join("\n---\n")
            );
        }
    }

    #[test]
    fn every_config_is_accepted_by_the_real_cores() {
        let mut problems = Vec::new();
        let mut validated = 0;
        for (sname, srv) in samples() {
            for (pname, routing) in presets() {
                for mode in [ConnectionMode::Proxy, ConnectionMode::Tun] {
                    let settings = settings(mode, routing.clone());
                    let mode_name = if mode == ConnectionMode::Tun {
                        "tun"
                    } else {
                        "proxy"
                    };

                    if xray_supports(&srv) {
                        let api = api_port(CoreKind::Xray, &settings);
                        let cfg = xray::full_config(&srv, &settings, api);
                        problems.extend(check_core(
                            CoreKind::Xray,
                            &cfg,
                            &format!("{sname}/{pname}/{mode_name}/xray"),
                        ));
                        validated += 1;
                    }
                    if singbox_supports(&srv, &settings) {
                        let api = api_port(CoreKind::SingBox, &settings);
                        let cfg = singbox::full_config(
                            &srv,
                            &settings,
                            mode == ConnectionMode::Tun,
                            None,
                            api,
                        );
                        problems.extend(check_core(
                            CoreKind::SingBox,
                            &cfg,
                            &format!("{sname}/{pname}/{mode_name}/sing-box"),
                        ));
                        validated += 1;
                    }
                }
            }
        }
        // A seam to notice accidental coverage loss; exact number not asserted.
        assert!(validated >= 17, "expected many configs, got {validated}");
        report(problems);
    }

    #[test]
    fn tun_xhttp_chain_configs_are_accepted() {
        let Some((_, srv)) = samples().into_iter().find(|(n, _)| *n == "vless-xhttp") else {
            return;
        };
        let mut problems = Vec::new();
        for (pname, routing) in presets() {
            let settings = settings(ConnectionMode::Tun, routing);
            let plans = super::plan(&srv, &settings);
            assert_eq!(
                plans.len(),
                2,
                "{pname}: xhttp chain must be xray + sing-box"
            );
            for plan in &plans {
                problems.extend(check_core(
                    plan.core,
                    &plan.config,
                    &format!("vless-xhttp/{pname}/tun-chain/{}", plan.core.binary_name()),
                ));
            }
        }
        report(problems);
    }
}

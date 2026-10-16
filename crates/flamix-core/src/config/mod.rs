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

use crate::model::{ConnectionMode, Protocol, ProviderRouting, Server, Settings};
use std::collections::BTreeMap;

/// Extra, provider-controlled inputs of one connect call.
///
/// `provider` is the routing profile of the subscription that owns the server (applied when
/// enabled); `overrides` are the Happ app-management keys (`socks-auth-*`, `http-auth-*`, ...).
/// The default context applies neither.
#[derive(Debug, Clone, Copy, Default)]
pub struct ConnectContext<'a> {
    pub provider: Option<&'a ProviderRouting>,
    pub overrides: Option<&'a BTreeMap<String, String>>,
}

/// Local inbound credentials (`socks-auth-*` / `http-auth-*`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LocalAuth {
    pub user: String,
    pub password: String,
}

/// Parse `<prefix>-auth-mode`/`-user`/`-password`. `prefix` is `socks` or `http`.
///
/// Happ's `<prefix>-auth-mode` is `auto` \| `manual` \| `from-json` \| `disable`; only
/// `manual` carries explicit credentials, and both user and password must be non-empty
/// (otherwise auth is treated as off, e.g. `auto`/`from-json`/`disable` or blank keys).
pub(crate) fn local_auth(
    prefix: &str,
    overrides: Option<&BTreeMap<String, String>>,
) -> Option<LocalAuth> {
    let map = overrides?;
    let mode = map.get(&format!("{prefix}-auth-mode"))?;
    if !mode.eq_ignore_ascii_case("manual") {
        return None;
    }
    let user = map.get(&format!("{prefix}-auth-user"))?;
    let password = map.get(&format!("{prefix}-auth-password"))?;
    if user.is_empty() || password.is_empty() {
        return None;
    }
    Some(LocalAuth {
        user: user.clone(),
        password: password.clone(),
    })
}

/// True when any Happ app override turns on local inbound authentication.
pub fn local_auth_enabled(overrides: Option<&BTreeMap<String, String>>) -> bool {
    local_auth("socks", overrides).is_some() || local_auth("http", overrides).is_some()
}

/// Probe URL used by the core-level failover balancer (`observatory` / `urltest`).
///
/// `settings.failover.probe_url` when set (non-blank), otherwise `settings.ping_url`.
pub(crate) fn failover_probe_url(settings: &Settings) -> String {
    let url = settings.failover.probe_url.trim();
    if url.is_empty() {
        settings.ping_url.clone()
    } else {
        url.to_string()
    }
}

/// Go-style duration string for a millisecond interval (accepted by both cores).
pub(crate) fn duration_ms(ms: u32) -> String {
    format!("{ms}ms")
}

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
    plan_with(server, settings, &ConnectContext::default())
}

/// Plan the processes for connecting to `server` with `settings` and a [`ConnectContext`]
/// carrying the selected subscription's provider routing and Happ app-overrides.
///
/// Provider routing (when present and enabled) is applied by [`xray::full_config_with`] /
/// [`singbox::full_config_with`]: its rules go before the user's custom rules, they replace
/// the user's preset, `global_proxy == false` sends unmatched traffic `direct`, and the
/// profile's DNS/hosts/fake-DNS/domain-strategy override the user settings.
pub fn plan_with(server: &Server, settings: &Settings, ctx: &ConnectContext) -> Vec<CorePlan> {
    plan_cluster(std::slice::from_ref(server), settings, ctx)
}

/// Plan the processes for a primary server plus its ordered backups.
///
/// `nodes[0]` is the primary; the rest are ordered backups. Core-level failover is applied
/// only when `settings.failover.enabled && nodes.len() > 1 && settings.mode == Proxy`;
/// otherwise the generated config is exactly the single-server config of [`plan_with`]
/// (so the disabled path never regresses). TUN mode never uses failover.
///
/// Provider routing/overrides ([`ConnectContext`]) belong to the subscription that owns the
/// **primary** node and are applied to the whole generated config; per-node provider context
/// is a later task.
pub fn plan_cluster(nodes: &[Server], settings: &Settings, ctx: &ConnectContext) -> Vec<CorePlan> {
    let Some(primary) = nodes.first() else {
        return Vec::new();
    };
    let failover =
        settings.failover.enabled && nodes.len() > 1 && settings.mode == ConnectionMode::Proxy;

    match settings.mode {
        ConnectionMode::Proxy => {
            if supported_by(primary).contains(&CoreKind::Xray) {
                let api_port = api_port(CoreKind::Xray, settings);
                let config = if failover {
                    xray::full_config_cluster_with(nodes, settings, api_port, ctx)
                } else {
                    xray::full_config_with(primary, settings, api_port, ctx)
                };
                vec![CorePlan {
                    core: CoreKind::Xray,
                    config,
                    elevated: false,
                    api_port,
                }]
            } else {
                let api_port = api_port(CoreKind::SingBox, settings);
                let config = if failover {
                    singbox::full_config_cluster_with(nodes, settings, false, None, api_port, ctx)
                } else {
                    singbox::full_config_with(primary, settings, false, None, api_port, ctx)
                };
                vec![CorePlan {
                    core: CoreKind::SingBox,
                    config,
                    elevated: false,
                    api_port,
                }]
            }
        }
        ConnectionMode::Tun => {
            // sing-box captures the TUN and forwards to the protocol outbound, or to a
            // local SOCKS inbound run by Xray when the transport is Xray-only (xhttp/kcp).
            let upstream_socks = singbox::outbound(primary, settings)
                .is_none()
                .then_some(settings.socks_port);
            let mut plans = Vec::new();
            if upstream_socks.is_some() {
                let api_port = api_port(CoreKind::Xray, settings);
                // This Xray process is an internal hop: sing-box (which captured the TUN)
                // owns routing and dials its local SOCKS inbound without credentials, so
                // provider routing/auth must NOT be applied here — only to the user-facing
                // inbounds of the TUN sing-box process below.
                plans.push(CorePlan {
                    core: CoreKind::Xray,
                    config: xray::full_config_with(
                        primary,
                        settings,
                        api_port,
                        &ConnectContext::default(),
                    ),
                    elevated: false,
                    api_port,
                });
            }
            let api_port = api_port(CoreKind::SingBox, settings);
            plans.push(CorePlan {
                core: CoreKind::SingBox,
                config: singbox::full_config_with(
                    primary,
                    settings,
                    true,
                    upstream_socks,
                    api_port,
                    ctx,
                ),
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
    use crate::model::{Credentials, FailoverMode, Network, Security, Transport};
    use serde_json::json;

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

    fn overrides(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn local_auth_only_for_manual_with_credentials() {
        let manual = overrides(&[
            ("socks-auth-mode", "manual"),
            ("socks-auth-user", "u"),
            ("socks-auth-password", "p"),
        ]);
        let auth = local_auth("socks", Some(&manual)).unwrap();
        assert_eq!(auth.user, "u");
        assert_eq!(auth.password, "p");
        assert!(local_auth_enabled(Some(&manual)));

        // Case-insensitive mode.
        let upper = overrides(&[
            ("socks-auth-mode", "MANUAL"),
            ("socks-auth-user", "u"),
            ("socks-auth-password", "p"),
        ]);
        assert!(local_auth("socks", Some(&upper)).is_some());

        // auto / from-json / disable do not enable auth.
        for mode in ["auto", "from-json", "disable"] {
            let m = overrides(&[
                ("socks-auth-mode", mode),
                ("socks-auth-user", "u"),
                ("socks-auth-password", "p"),
            ]);
            assert!(local_auth("socks", Some(&m)).is_none(), "mode={mode}");
            assert!(!local_auth_enabled(Some(&m)), "mode={mode}");
        }
    }

    #[test]
    fn local_auth_requires_non_empty_credentials() {
        let missing = overrides(&[("socks-auth-mode", "manual")]);
        assert!(local_auth("socks", Some(&missing)).is_none());

        let blank_user = overrides(&[
            ("socks-auth-mode", "manual"),
            ("socks-auth-user", ""),
            ("socks-auth-password", "p"),
        ]);
        assert!(local_auth("socks", Some(&blank_user)).is_none());

        let blank_password = overrides(&[
            ("socks-auth-mode", "manual"),
            ("socks-auth-user", "u"),
            ("socks-auth-password", ""),
        ]);
        assert!(local_auth("socks", Some(&blank_password)).is_none());
        assert!(!local_auth_enabled(Some(&blank_password)));

        assert!(local_auth("http", None).is_none());
        assert!(!local_auth_enabled(None));
    }

    #[test]
    fn tun_chain_xray_ignores_provider_routing_and_auth() {
        // The provider asks for global-direct routing and local auth.
        let provider = ProviderRouting {
            name: "p".into(),
            enabled: true,
            global_proxy: false,
            domain_strategy: Some("AsIs".into()),
            ..ProviderRouting::default()
        };
        let ov = overrides(&[
            ("socks-auth-mode", "manual"),
            ("socks-auth-user", "u"),
            ("socks-auth-password", "p"),
        ]);
        let ctx = ConnectContext {
            provider: Some(&provider),
            overrides: Some(&ov),
        };
        let settings = Settings {
            mode: ConnectionMode::Tun,
            ..Settings::default()
        };
        let plans = plan_with(&vless_xhttp(), &settings, &ctx);
        assert_eq!(plans.len(), 2);
        // Chained Xray must be built with the default context: no provider routing, no auth.
        let expected = xray::full_config(&vless_xhttp(), &settings, plans[0].api_port);
        assert_eq!(plans[0].config, expected);
        assert_eq!(plans[0].config["inbounds"][0]["settings"]["auth"], "noauth");
        // No conditionless provider catch-all rule, and no provider domain strategy.
        let rules = plans[0].config["routing"]["rules"].as_array().unwrap();
        assert!(!rules.iter().any(|r| r.get("domain").is_none()
            && r.get("ip").is_none()
            && r.get("inboundTag").is_none()));
        assert_eq!(plans[0].config["routing"]["domainStrategy"], "IPIfNonMatch");
    }

    fn trojan_ws() -> Server {
        server(
            Credentials::Trojan {
                password: "secret".into(),
            },
            Transport {
                network: Network::Ws,
                path: Some("/ws".into()),
                host: Some("ws.example".into()),
                ..Transport::default()
            },
        )
    }

    fn hy2() -> Server {
        server(
            Credentials::Hysteria2 {
                password: "pw".into(),
                obfs: None,
                obfs_password: None,
                ports: None,
                up_mbps: None,
                down_mbps: None,
            },
            Transport::default(),
        )
    }

    fn failover_settings() -> Settings {
        let mut settings = Settings::default();
        settings.failover.enabled = true;
        settings.failover.probe_interval_ms = 2000;
        settings
    }

    #[test]
    fn plan_cluster_one_node_matches_plan() {
        // One node, failover on or off: byte-for-byte the single-server plan.
        let single = plan(&vless_xhttp(), &Settings::default());
        assert_eq!(
            plan_cluster(
                &[vless_xhttp()],
                &Settings::default(),
                &ConnectContext::default()
            ),
            single
        );
        assert_eq!(
            plan_cluster(
                &[vless_xhttp()],
                &failover_settings(),
                &ConnectContext::default()
            ),
            single
        );
    }

    #[test]
    fn plan_cluster_empty_is_empty() {
        assert!(plan_cluster(&[], &failover_settings(), &ConnectContext::default()).is_empty());
    }

    #[test]
    fn plan_cluster_disabled_path_is_unchanged() {
        let nodes = [vless_xhttp(), trojan_ws()];
        let settings = Settings::default();
        assert_eq!(
            plan_cluster(&nodes, &settings, &ConnectContext::default()),
            plan(&nodes[0], &settings)
        );
    }

    #[test]
    fn plan_cluster_tun_ignores_failover() {
        let nodes = [vless_xhttp(), trojan_ws()];
        let settings = Settings {
            mode: ConnectionMode::Tun,
            failover: crate::model::Failover {
                enabled: true,
                ..crate::model::Failover::default()
            },
            ..Settings::default()
        };
        assert_eq!(
            plan_cluster(&nodes, &settings, &ConnectContext::default()),
            plan(&nodes[0], &settings)
        );
    }

    #[test]
    fn plan_cluster_xray_balancer_and_observatory() {
        let nodes = [vless_xhttp(), trojan_ws()];
        let mut settings = failover_settings();
        settings.failover.probe_url = "https://probe.example/204".into();

        let plans = plan_cluster(&nodes, &settings, &ConnectContext::default());
        assert_eq!(plans.len(), 1);
        let cfg = &plans[0].config;
        let tags: Vec<&str> = cfg["outbounds"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|o| o["tag"].as_str())
            .collect();
        assert_eq!(tags, ["node-0", "node-1", "direct", "block"]);
        // The routing generator is unchanged: custom rules still target tag `proxy`.
        let balancer = &cfg["routing"]["balancers"][0];
        assert_eq!(balancer["tag"], json!("proxy"));
        assert_eq!(balancer["selector"], json!(["node-0", "node-1"]));
        assert_eq!(balancer["strategy"]["type"], json!("leastPing"));
        assert_eq!(
            cfg["observatory"]["subjectSelector"],
            json!(["node-0", "node-1"])
        );
        assert_eq!(
            cfg["observatory"]["probeUrl"],
            json!("https://probe.example/204")
        );
        assert_eq!(cfg["observatory"]["probeInterval"], json!("2000ms"));
        assert_eq!(cfg["observatory"]["enableConcurrency"], json!(true));

        // LoadBalance -> roundRobin.
        settings.failover.mode = FailoverMode::LoadBalance;
        let cfg = &plan_cluster(&nodes, &settings, &ConnectContext::default())[0].config;
        assert_eq!(
            cfg["routing"]["balancers"][0]["strategy"]["type"],
            json!("roundRobin")
        );

        // Blank probe_url falls back to ping_url.
        settings.failover.probe_url = "  ".into();
        let cfg = &plan_cluster(&nodes, &settings, &ConnectContext::default())[0].config;
        assert_eq!(cfg["observatory"]["probeUrl"], json!(settings.ping_url));
    }

    #[test]
    fn plan_cluster_singbox_urltest() {
        let nodes = [
            hy2(),
            server(
                Credentials::Tuic {
                    uuid: "33333333-3333-3333-3333-333333333333".into(),
                    password: "secret".into(),
                    congestion_control: None,
                    udp_relay_mode: None,
                },
                Transport::default(),
            ),
        ];
        let mut settings = failover_settings();
        settings.failover.tolerance_ms = 80;
        let plans = plan_cluster(&nodes, &settings, &ConnectContext::default());
        assert_eq!(plans.len(), 1);
        assert_eq!(plans[0].core, CoreKind::SingBox);
        let cfg = &plans[0].config;
        let outbounds = cfg["outbounds"].as_array().unwrap();
        assert_eq!(outbounds[0]["type"], json!("urltest"));
        assert_eq!(outbounds[0]["tag"], json!("proxy"));
        assert_eq!(outbounds[0]["outbounds"], json!(["node-0", "node-1"]));
        assert_eq!(outbounds[0]["url"], json!(settings.ping_url));
        assert_eq!(outbounds[0]["interval"], json!("2000ms"));
        assert_eq!(outbounds[0]["tolerance"], json!(80));
        assert_eq!(outbounds[1]["tag"], json!("node-0"));
        assert_eq!(outbounds[2]["tag"], json!("node-1"));
        // route.final and DNS detour keep pointing at `proxy`.
        assert_eq!(cfg["route"]["final"], json!("proxy"));
        assert_eq!(cfg["dns"]["servers"][0]["detour"], json!("proxy"));
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
        Credentials, Dns, FailoverMode, Network, Routing, RoutingPreset, RoutingRule, RuleAction,
        Security, Transport,
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

    /// Primary `vless+reality` + backup `trojan+ws`: two transports for the cluster tests.
    fn failover_nodes() -> Vec<Server> {
        let primary = samples()
            .into_iter()
            .find(|(name, _)| *name == "vless-reality")
            .map(|(_, server)| server)
            .expect("vless-reality sample");
        let backup = server(
            Credentials::Trojan {
                password: "secret".into(),
            },
            Transport {
                network: Network::Ws,
                path: Some("/ws".into()),
                host: Some("ws.example".into()),
                ..Transport::default()
            },
            tls("ws.example"),
        );
        vec![primary, backup]
    }

    #[test]
    fn failover_cluster_configs_are_accepted_by_the_real_cores() {
        if std::env::var("FLAMIX_XRAY_BIN").is_err() && std::env::var("FLAMIX_SINGBOX_BIN").is_err()
        {
            return;
        }
        let nodes = failover_nodes();
        let ctx = ConnectContext::default();
        let mut problems = Vec::new();
        for (label, mode) in [
            ("failover", FailoverMode::Failover),
            ("load-balance", FailoverMode::LoadBalance),
        ] {
            let mut settings = settings(ConnectionMode::Proxy, Routing::default());
            settings.failover.enabled = true;
            settings.failover.mode = mode;
            let api = api_port(CoreKind::Xray, &settings);
            let cfg = xray::full_config_cluster_with(&nodes, &settings, api, &ctx);
            problems.extend(check_core(
                CoreKind::Xray,
                &cfg,
                &format!("multi-node/{label}/xray"),
            ));
            let api = api_port(CoreKind::SingBox, &settings);
            let cfg = singbox::full_config_cluster_with(&nodes, &settings, false, None, api, &ctx);
            problems.extend(check_core(
                CoreKind::SingBox,
                &cfg,
                &format!("multi-node/{label}/sing-box"),
            ));
        }
        // Failover off: byte-for-byte the single-server config.
        let settings = settings(ConnectionMode::Proxy, Routing::default());
        let api = api_port(CoreKind::Xray, &settings);
        assert_eq!(
            xray::full_config_cluster_with(&nodes, &settings, api, &ctx),
            xray::full_config_with(&nodes[0], &settings, api, &ctx),
        );
        // Failover on but a single node: still the single-server config.
        let mut enabled = settings.clone();
        enabled.failover.enabled = true;
        assert_eq!(
            xray::full_config_cluster_with(&nodes[0..1], &enabled, api, &ctx),
            xray::full_config_with(&nodes[0], &enabled, api, &ctx),
        );
        report(problems);
    }

    /// Provider routing profiles with every feature (rules, final, dns, hosts, fake-dns, geo).
    fn provider_routings() -> Vec<(&'static str, ProviderRouting)> {
        let global_direct = ProviderRouting {
            name: "global-direct".into(),
            enabled: true,
            global_proxy: false,
            rules: vec![
                RoutingRule {
                    domains: vec!["geosite:google".into(), "domain:ads.example".into()],
                    ips: vec![],
                    processes: vec![],
                    network: None,
                    ports: None,
                    action: RuleAction::Direct,
                    enabled: true,
                },
                RoutingRule {
                    domains: vec![],
                    ips: vec!["geoip:ru".into()],
                    processes: vec![],
                    network: Some("tcp".into()),
                    ports: Some("443".into()),
                    action: RuleAction::Proxy,
                    enabled: true,
                },
                RoutingRule {
                    domains: vec!["geosite:category-ads-all".into()],
                    ips: vec![],
                    processes: vec![],
                    network: None,
                    ports: None,
                    action: RuleAction::Block,
                    enabled: true,
                },
            ],
            domain_strategy: Some("IPIfNonMatch".into()),
            dns: Some(Dns {
                remote: "https://1.1.1.1/dns-query".into(),
                direct: "https://77.88.8.8/dns-query".into(),
                fake_ip: false,
            }),
            dns_hosts: BTreeMap::from([("host.example".into(), "10.0.0.7".into())]),
            geoip_url: Some("https://cdn.example/geoip.dat".into()),
            geosite_url: Some("https://cdn.example/geosite.dat".into()),
            ..ProviderRouting::default()
        };
        let global_proxy = ProviderRouting {
            name: "global-proxy-fake".into(),
            enabled: true,
            global_proxy: true,
            rules: vec![RoutingRule {
                domains: vec!["geosite:private".into()],
                ips: vec!["geoip:private".into()],
                processes: vec![],
                network: None,
                ports: None,
                action: RuleAction::Direct,
                enabled: true,
            }],
            dns: Some(Dns {
                remote: "https://9.9.9.9/dns-query".into(),
                direct: "https://77.88.8.8/dns-query".into(),
                fake_ip: true,
            }),
            fake_dns: true,
            ..ProviderRouting::default()
        };
        vec![
            ("global-direct", global_direct),
            ("global-proxy-fake", global_proxy),
        ]
    }

    /// Happ app-override sets: none, and local inbound auth.
    fn override_sets() -> Vec<(&'static str, Option<BTreeMap<String, String>>)> {
        vec![
            ("no-override", None),
            (
                "auth",
                Some(BTreeMap::from([
                    ("socks-auth-mode".to_string(), "manual".to_string()),
                    ("socks-auth-user".to_string(), "user".to_string()),
                    ("socks-auth-password".to_string(), "pass".to_string()),
                    ("http-auth-mode".to_string(), "manual".to_string()),
                    ("http-auth-user".to_string(), "user".to_string()),
                    ("http-auth-password".to_string(), "pass".to_string()),
                ])),
            ),
        ]
    }

    #[test]
    fn provider_routing_configs_are_accepted_by_the_real_cores() {
        if std::env::var("FLAMIX_XRAY_BIN").is_err() && std::env::var("FLAMIX_SINGBOX_BIN").is_err()
        {
            return;
        }
        let mut problems = Vec::new();
        let mut validated = 0;
        for (sname, srv) in samples() {
            for (pname, provider) in provider_routings() {
                for (oname, overrides) in override_sets() {
                    for mode in [ConnectionMode::Proxy, ConnectionMode::Tun] {
                        let settings = settings(mode, Routing::default());
                        let ctx = ConnectContext {
                            provider: Some(&provider),
                            overrides: overrides.as_ref(),
                        };
                        let mode_name = if mode == ConnectionMode::Tun {
                            "tun"
                        } else {
                            "proxy"
                        };
                        if xray_supports(&srv) {
                            let api = api_port(CoreKind::Xray, &settings);
                            let cfg = xray::full_config_with(&srv, &settings, api, &ctx);
                            problems.extend(check_core(
                                CoreKind::Xray,
                                &cfg,
                                &format!("{sname}/{pname}/{oname}/{mode_name}/xray"),
                            ));
                            validated += 1;
                        }
                        if singbox_supports(&srv, &settings) {
                            let api = api_port(CoreKind::SingBox, &settings);
                            let cfg = singbox::full_config_with(
                                &srv,
                                &settings,
                                mode == ConnectionMode::Tun,
                                None,
                                api,
                                &ctx,
                            );
                            problems.extend(check_core(
                                CoreKind::SingBox,
                                &cfg,
                                &format!("{sname}/{pname}/{oname}/{mode_name}/sing-box"),
                            ));
                            validated += 1;
                        }
                    }
                }
            }
        }
        assert!(validated > 0, "no provider configs validated");
        report(problems);
    }

    #[test]
    fn provider_tun_chain_configs_are_accepted_by_the_real_cores() {
        if std::env::var("FLAMIX_XRAY_BIN").is_err() && std::env::var("FLAMIX_SINGBOX_BIN").is_err()
        {
            return;
        }
        let Some((_, srv)) = samples().into_iter().find(|(n, _)| *n == "vless-xhttp") else {
            return;
        };
        let mut problems = Vec::new();
        for (pname, provider) in provider_routings() {
            for (oname, overrides) in override_sets() {
                let ctx = ConnectContext {
                    provider: Some(&provider),
                    overrides: overrides.as_ref(),
                };
                let settings = settings(ConnectionMode::Tun, Routing::default());
                let plans = super::plan_with(&srv, &settings, &ctx);
                assert_eq!(
                    plans.len(),
                    2,
                    "{pname}/{oname}: xhttp chain must be xray + sing-box"
                );
                for plan in &plans {
                    problems.extend(check_core(
                        plan.core,
                        &plan.config,
                        &format!(
                            "vless-xhttp/{pname}/{oname}/tun-chain/{}",
                            plan.core.binary_name()
                        ),
                    ));
                }
            }
        }
        report(problems);
    }
}

//! Proxy core configuration generators. OWNER: task T03.
//!
//! Flamix drives external cores, like v2rayN (Xray-core and sing-box by default; mihomo when
//! `settings.core_override == "mihomo"`):
//! - **Xray-core** for Proxy mode and for every protocol it supports (vless incl. reality/xhttp,
//!   vmess, trojan, shadowsocks, socks, http, wireguard);
//! - **sing-box** for hysteria2 / tuic (Proxy mode) and for TUN mode: its `tun` inbound
//!   captures traffic and forwards it to the selected protocol's outbound directly (sing-box
//!   protocols) or to Xray's local SOCKS inbound (Xray-only features like xhttp).

pub mod mihomo;
mod routing;
pub(crate) mod ru_routing;
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

/// True when the local inbounds will require authentication under the given settings and
/// provider/Happ overrides.
///
/// This is the union of the two auth sources ([`effective_local_auth`]): a provider
/// `socks-auth-*`/`http-auth-*` override, or the user `local_auth_user` + `local_auth_pass`
/// when both are non-empty. Callers that only want to know "is auth on?" (e.g. the system-proxy
/// decision in the app) should use this rather than re-deriving the precedence.
pub fn settings_auth_enabled(
    settings: &Settings,
    overrides: Option<&BTreeMap<String, String>>,
) -> bool {
    local_auth_enabled(overrides)
        || (!settings.local_auth_user.is_empty() && !settings.local_auth_pass.is_empty())
}

/// A copy of `settings` with the user local-inbound credentials stripped.
///
/// Used for the **internal** Xray hop of TUN mode: sing-box (which captured the TUN) dials
/// this hop's SOCKS inbound without credentials, so a user `local_auth_user`/`local_auth_pass`
/// must not turn the internal listener into an authenticated one (it would make TUN + an
/// Xray-only transport + user auth unable to connect). Provider overrides are already absent
/// there because the hop is built with [`ConnectContext::default`].
fn internal_hop_settings(settings: &Settings) -> Settings {
    Settings {
        local_auth_user: String::new(),
        local_auth_pass: String::new(),
        ..settings.clone()
    }
}

/// Local inbound credentials for `prefix`, with the provider/Happ override taking precedence
/// over the user setting.
///
/// Precedence (documented, tested):
/// 1. A valid Happ `socks-auth-*`/`http-auth-*` `manual` override for this prefix wins.
/// 2. Otherwise, `settings.local_auth_user` + `settings.local_auth_pass` when both are
///    non-empty.
/// 3. Otherwise no auth.
///
/// The override is per prefix: a provider that authenticates only its SOCKS inbound still
/// lets the user credentials apply to the HTTP inbound, and vice versa.
pub(crate) fn effective_local_auth(
    prefix: &str,
    settings: &Settings,
    overrides: Option<&BTreeMap<String, String>>,
) -> Option<LocalAuth> {
    if let Some(auth) = local_auth(prefix, overrides) {
        return Some(auth);
    }
    if !settings.local_auth_user.is_empty() && !settings.local_auth_pass.is_empty() {
        return Some(LocalAuth {
            user: settings.local_auth_user.clone(),
            password: settings.local_auth_pass.clone(),
        });
    }
    None
}

/// Fallback probe URL for the core balancer when neither configured URL is set.
pub(crate) const DEFAULT_PROBE_URL: &str = "https://www.gstatic.com/generate_204";

/// Lower bound for the balancer probe interval; a smaller (or zero) value would flood the
/// health probes, so it is clamped up.
pub(crate) const MIN_PROBE_INTERVAL_MS: u32 = 15000;

/// Probe URL used by the core-level failover balancer (`observatory` / `urltest`).
///
/// `settings.failover.probe_url` when set (non-blank), else `settings.ping_url`, else a
/// non-empty constant so the generated config always carries a valid URL.
pub(crate) fn failover_probe_url(settings: &Settings) -> String {
    let url = settings.failover.probe_url.trim();
    if !url.is_empty() {
        return url.to_string();
    }
    let ping = settings.ping_url.trim();
    if !ping.is_empty() {
        return ping.to_string();
    }
    DEFAULT_PROBE_URL.to_string()
}

/// Probe interval clamped to at least [`MIN_PROBE_INTERVAL_MS`].
pub(crate) fn probe_interval_ms(settings: &Settings) -> u32 {
    settings
        .failover
        .probe_interval_ms
        .max(MIN_PROBE_INTERVAL_MS)
}

/// Go-style duration string for a millisecond interval (accepted by both cores).
pub(crate) fn duration_ms(ms: u32) -> String {
    format!("{ms}ms")
}

/// True when `kind` can run `server` as a proxy outbound inside a failover cluster.
///
/// Xray cannot run hysteria2/tuic — [`xray::outbound`] would emit a `freedom`/direct hop, and
/// `leastPing` would then rank that leak as the fastest node, so such a node is never
/// included. sing-box cannot run xhttp/kcp/quic and needs WireGuard as an endpoint, not a
/// urltest member.
fn node_supports(kind: CoreKind, server: &Server, settings: &Settings) -> bool {
    match kind {
        CoreKind::Xray => supported_by(server).contains(&CoreKind::Xray),
        CoreKind::SingBox => {
            supported_by(server).contains(&CoreKind::SingBox)
                && singbox::clusterable(server, settings)
        }
        CoreKind::Mihomo => mihomo::supports(server, settings),
    }
}

/// Which external core binary to run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoreKind {
    Xray,
    SingBox,
    /// mihomo (Clash.Meta); selected explicitly with `core_override = "mihomo"`.
    Mihomo,
}

impl CoreKind {
    /// Executable base name (without `.exe`).
    pub fn binary_name(self) -> &'static str {
        match self {
            Self::Xray => "xray",
            Self::SingBox => "sing-box",
            Self::Mihomo => "mihomo",
        }
    }
}

/// One process to start: which core and the config to feed it (JSON for Xray/sing-box; for
/// mihomo the same value is written as YAML, see [`mihomo::render_yaml`]).
#[derive(Debug, Clone, PartialEq)]
pub struct CorePlan {
    pub core: CoreKind,
    pub config: serde_json::Value,
    /// Needs admin/root (TUN).
    pub elevated: bool,
    /// Stats endpoint: Xray gRPC API port (`xray api statsquery`) or sing-box clash_api port.
    pub api_port: u16,
}

/// Port of the stats API for a core: Xray = `socks_port + 10`, sing-box = `socks_port + 11`,
/// mihomo (external-controller) = `socks_port + 12`.
pub fn api_port(kind: CoreKind, settings: &Settings) -> u16 {
    let off = match kind {
        CoreKind::Xray => 10,
        CoreKind::SingBox => 11,
        CoreKind::Mihomo => 12,
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
/// only when `settings.failover.enabled && settings.mode == Proxy` and, after filtering,
/// more than one node remains; otherwise the generated config is exactly the single-server
/// config of [`plan_with`] (so the disabled path never regresses). TUN mode never uses failover.
///
/// Unsupported-node policy (identical for both cores): backups the chosen core cannot proxy
/// are **dropped, and the compatible ones are kept**, with a `tracing::warn!` naming the
/// dropped nodes. The chosen core is the one that can run the primary, so it is always kept;
/// if no compatible backup remains, the cluster collapses back to the single-server config.
/// A node is *never* emitted as a `direct`/`freedom` hop inside a cluster (that would leak).
///
/// Provider routing/overrides ([`ConnectContext`]) belong to the subscription that owns the
/// **primary** node and are applied to the whole generated config; per-node provider context
/// is a later task.
pub fn plan_cluster(nodes: &[Server], settings: &Settings, ctx: &ConnectContext) -> Vec<CorePlan> {
    let Some(primary) = nodes.first() else {
        return Vec::new();
    };

    // `core_override = "mihomo"`: one mihomo process (Proxy and TUN alike). A server mihomo
    // cannot express falls back to the automatic choice with a warning (never a leak).
    if settings.core_override == "mihomo" {
        let api_port = api_port(CoreKind::Mihomo, settings);
        if let Some(config) = mihomo::full_config_with(primary, settings, api_port, ctx) {
            if nodes.len() > 1 && settings.failover.enabled {
                tracing::warn!("core_override=mihomo: failover clusters are not supported; using the primary only");
            }
            return vec![CorePlan {
                core: CoreKind::Mihomo,
                config,
                elevated: settings.mode == ConnectionMode::Tun,
                api_port,
            }];
        }
        tracing::warn!(
            "core_override=mihomo: server '{}' is not mihomo-capable (transport/protocol); using auto",
            primary.name
        );
    }

    match settings.mode {
        ConnectionMode::Proxy => {
            // The core is picked from the primary; backups it cannot proxy are dropped so a
            // cluster never includes a `direct`/`freedom` node that `leastPing` would prefer.
            let auto = if supported_by(primary).contains(&CoreKind::Xray) {
                CoreKind::Xray
            } else {
                CoreKind::SingBox
            };
            // `settings.core_override`: "auto" keeps the automatic choice; a forced core that
            // cannot express the primary falls back to `auto` with a warning (never a leak).
            let kind = match settings.core_override.as_str() {
                "xray" if supported_by(primary).contains(&CoreKind::Xray) => CoreKind::Xray,
                "xray" => {
                    tracing::warn!(
                        "core_override=xray: server '{}' is not Xray-capable; using {}",
                        primary.name,
                        auto.binary_name()
                    );
                    auto
                }
                "sing-box" if singbox::outbound(primary, settings).is_some() => CoreKind::SingBox,
                "sing-box" => {
                    tracing::warn!(
                        "core_override=sing-box: server '{}' is not sing-box-capable; using {}",
                        primary.name,
                        auto.binary_name()
                    );
                    auto
                }
                _ => auto,
            };
            let compatible: Vec<Server> = nodes
                .iter()
                .filter(|node| node_supports(kind, node, settings))
                .cloned()
                .collect();
            let dropped: Vec<&str> = nodes
                .iter()
                .filter(|node| !node_supports(kind, node, settings))
                .map(|node| node.name.as_str())
                .collect();
            if !dropped.is_empty() {
                tracing::warn!(
                    "failover: dropping {} node(s) that {} cannot proxy: {}",
                    dropped.len(),
                    kind.binary_name(),
                    dropped.join(", ")
                );
            }
            // `compatible[0]` is the primary: `kind` was chosen because the primary supports it.
            let failover = settings.failover.enabled && compatible.len() > 1;
            match kind {
                CoreKind::Xray => {
                    let api_port = api_port(CoreKind::Xray, settings);
                    let config = if failover {
                        xray::full_config_cluster_with(&compatible, settings, api_port, ctx)
                    } else {
                        xray::full_config_with(primary, settings, api_port, ctx)
                    };
                    vec![CorePlan {
                        core: CoreKind::Xray,
                        config,
                        elevated: false,
                        api_port,
                    }]
                }
                // `kind` is only ever Xray or SingBox here (mihomo returned above).
                CoreKind::SingBox | CoreKind::Mihomo => {
                    let api_port = api_port(CoreKind::SingBox, settings);
                    let config = if failover {
                        singbox::full_config_cluster_with(
                            &compatible,
                            settings,
                            false,
                            None,
                            api_port,
                            ctx,
                        )
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
        }
        ConnectionMode::Tun | ConnectionMode::Tproxy => {
            // sing-box always captures the privileged inbound — the only core with `tun` and
            // `tproxy` inbounds — and forwards to the protocol outbound, or to a local SOCKS
            // inbound run by Xray when the transport is Xray-only (xhttp/kcp/quic) — or when
            // `core_override` forces it. TPROXY (Linux-only) and TUN share this plan;
            // `singbox::build_config` picks the inbound shape from `settings.mode`.
            //
            // `core_override` selection:
            // - "xray": run the internal Xray SOCKS hop for Xray-capable servers (falls back to
            //   the native sing-box outbound with a warning for hysteria2/tuic);
            // - "sing-box"/"auto": native sing-box outbound when it can express the server,
            //   else the Xray hop (xhttp/kcp/quic have no sing-box outbound).
            let native = singbox::outbound(primary, settings).is_some();
            let xray_capable = supported_by(primary).contains(&CoreKind::Xray);
            let force_xray = settings.core_override == "xray";
            let upstream_socks = if native {
                if force_xray && xray_capable {
                    Some(settings.socks_port)
                } else {
                    if force_xray {
                        tracing::warn!(
                            "core_override=xray: server '{}' is not Xray-capable in TUN; using sing-box",
                            primary.name
                        );
                    }
                    None
                }
            } else {
                if settings.core_override == "sing-box" {
                    tracing::warn!(
                        "core_override=sing-box: server '{}' needs the Xray hop in TUN; ignoring",
                        primary.name
                    );
                }
                Some(settings.socks_port)
            };
            let mut plans = Vec::new();
            if upstream_socks.is_some() {
                let api_port = api_port(CoreKind::Xray, settings);
                // This Xray process is an internal hop: sing-box (which captured the TUN)
                // owns routing and dials its local SOCKS inbound without credentials, so
                // provider routing/auth and the user `local_auth_*` must NOT reach it —
                // only the user-facing inbounds of the TUN sing-box process below.
                plans.push(CorePlan {
                    core: CoreKind::Xray,
                    config: xray::full_config_with(
                        primary,
                        &internal_hop_settings(settings),
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
    fn core_override_mihomo_plans_one_process() {
        let ws = server(
            Credentials::Trojan {
                password: "p".into(),
            },
            Transport::default(),
        );
        let mut settings = Settings {
            core_override: "mihomo".into(),
            ..Settings::default()
        };
        let plans = plan(&ws, &settings);
        assert_eq!(plans.len(), 1);
        assert_eq!(plans[0].core, CoreKind::Mihomo);
        assert!(!plans[0].elevated);
        assert_eq!(plans[0].api_port, 10820);
        assert_eq!(plans[0].config["mixed-port"], json!(10808));
        settings.mode = ConnectionMode::Tun;
        let plans = plan(&ws, &settings);
        assert_eq!(plans.len(), 1);
        assert_eq!(plans[0].core, CoreKind::Mihomo);
        assert!(plans[0].elevated);
        assert_eq!(plans[0].config["tun"]["enable"], json!(true));
        assert_eq!(CoreKind::Mihomo.binary_name(), "mihomo");
    }

    #[test]
    fn core_override_mihomo_falls_back_to_auto() {
        // xhttp is not expressible in mihomo: auto picks Xray in Proxy mode.
        let settings = Settings {
            core_override: "mihomo".into(),
            ..Settings::default()
        };
        let plans = plan(&vless_xhttp(), &settings);
        assert_eq!(plans.len(), 1);
        assert_eq!(plans[0].core, CoreKind::Xray);
        // Auto keeps today's behaviour for a mihomo-capable server.
        let plans = plan(
            &server(
                Credentials::Trojan {
                    password: "p".into(),
                },
                Transport::default(),
            ),
            &Settings::default(),
        );
        assert_eq!(plans[0].core, CoreKind::Xray);
    }

    #[test]
    fn rf_preset_falls_back_to_direct_per_core() {
        let s = server(
            Credentials::Trojan {
                password: "p".into(),
            },
            Transport::default(),
        );
        // Xray: an explicit catch-all to `direct` closes the rules.
        let xr = xray::full_config(&s, &Settings::default(), 1);
        let last = xr["routing"]["rules"]
            .as_array()
            .unwrap()
            .last()
            .unwrap()
            .clone();
        assert_eq!(last["outboundTag"], json!("direct"));
        assert_eq!(last["network"], json!("tcp,udp"));
        // sing-box: route.final = direct, DNS detour stays the proxy.
        let sb = singbox::full_config(&s, &Settings::default(), false, None, 1);
        assert_eq!(sb["route"]["final"], json!("direct"));
        assert_eq!(
            sb["dns"]["servers"]
                .as_array()
                .unwrap()
                .iter()
                .find(|d| d["tag"] == "remote")
                .unwrap()["detour"],
            json!("proxy")
        );
        // Other presets keep "unmatched goes to proxy".
        let mut g = Settings::default();
        g.routing.preset = crate::model::RoutingPreset::Global;
        let sb = singbox::full_config(&s, &g, false, None, 1);
        assert_eq!(sb["route"]["final"], json!("proxy"));
        let xr = xray::full_config(&s, &g, 1);
        assert!(xr["routing"]["rules"]
            .as_array()
            .unwrap()
            .last()
            .unwrap()
            .get("network")
            .is_none());
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
    fn effective_local_auth_provider_wins_then_user_then_none() {
        let manual = overrides(&[
            ("socks-auth-mode", "manual"),
            ("socks-auth-user", "prov"),
            ("socks-auth-password", "secret"),
        ]);
        let user = Settings {
            local_auth_user: "user".into(),
            local_auth_pass: "pass".into(),
            ..Settings::default()
        };

        // Provider override wins for its prefix...
        let auth = effective_local_auth("socks", &user, Some(&manual)).unwrap();
        assert_eq!(auth.user, "prov");
        assert_eq!(auth.password, "secret");
        // ...but not for the other prefix: the user setting still applies there.
        let auth = effective_local_auth("http", &user, Some(&manual)).unwrap();
        assert_eq!(auth.user, "user");
        assert_eq!(auth.password, "pass");

        // No override: the user setting is used.
        let auth = effective_local_auth("socks", &user, None).unwrap();
        assert_eq!(auth.user, "user");

        // A half-set user credential is ignored.
        let half = Settings {
            local_auth_user: "user".into(),
            local_auth_pass: String::new(),
            ..Settings::default()
        };
        assert!(effective_local_auth("socks", &half, None).is_none());

        // Defaults: no auth at all.
        assert!(effective_local_auth("socks", &Settings::default(), None).is_none());
    }

    #[test]
    fn settings_auth_enabled_unions_provider_and_user() {
        let manual = overrides(&[
            ("socks-auth-mode", "manual"),
            ("socks-auth-user", "prov"),
            ("socks-auth-password", "secret"),
        ]);
        let user = Settings {
            local_auth_user: "user".into(),
            local_auth_pass: "pass".into(),
            ..Settings::default()
        };

        // Neither source: off.
        assert!(!settings_auth_enabled(&Settings::default(), None));
        // Provider override only.
        assert!(settings_auth_enabled(&Settings::default(), Some(&manual)));
        // User setting only.
        assert!(settings_auth_enabled(&user, None));
        // Both.
        assert!(settings_auth_enabled(&user, Some(&manual)));

        // A half-set user credential does not count.
        let half = Settings {
            local_auth_user: "user".into(),
            ..Settings::default()
        };
        assert!(!settings_auth_enabled(&half, None));
    }

    #[test]
    fn core_override_forces_and_falls_back() {
        // One Xray-capable, one sing-box-capable server.
        let ws = server(
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
        let hy2 = server(
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

        // "auto" keeps today's choice.
        assert_eq!(plan(&ws, &Settings::default())[0].core, CoreKind::Xray);
        assert_eq!(plan(&hy2, &Settings::default())[0].core, CoreKind::SingBox);

        // "sing-box" forces sing-box for a server it can express...
        let sb = Settings {
            core_override: "sing-box".into(),
            ..Settings::default()
        };
        assert_eq!(plan(&ws, &sb)[0].core, CoreKind::SingBox);
        // ... and falls back to auto (Xray) for an xhttp server sing-box cannot express.
        let xhttp = vless_xhttp();
        assert_eq!(plan(&xhttp, &sb)[0].core, CoreKind::Xray);

        // "xray" forces Xray when possible, else falls back to auto (sing-box).
        let xray = Settings {
            core_override: "xray".into(),
            ..Settings::default()
        };
        assert_eq!(plan(&ws, &xray)[0].core, CoreKind::Xray);
        assert_eq!(plan(&hy2, &xray)[0].core, CoreKind::SingBox);

        // Unknown values are treated as "auto".
        let bogus = Settings {
            core_override: "whatever".into(),
            ..Settings::default()
        };
        assert_eq!(plan(&hy2, &bogus)[0].core, CoreKind::SingBox);
    }

    #[test]
    fn core_override_in_tun_mode() {
        let ws = server(
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
        let tun = |core_override: &str| Settings {
            mode: ConnectionMode::Tun,
            core_override: core_override.into(),
            ..Settings::default()
        };

        // sing-box always captures the TUN; auto runs the native sing-box outbound.
        let plans = plan(&ws, &tun("auto"));
        assert_eq!(plans.len(), 1);
        assert_eq!(plans[0].core, CoreKind::SingBox);
        assert_eq!(plans[0].config["outbounds"][0]["type"], "vless");

        // "sing-box" stays native.
        let plans = plan(&ws, &tun("sing-box"));
        assert_eq!(plans.len(), 1);
        assert_eq!(plans[0].config["outbounds"][0]["type"], "vless");

        // "xray" forces the internal Xray hop for an Xray-capable server.
        let plans = plan(&ws, &tun("xray"));
        assert_eq!(plans.len(), 2);
        assert_eq!(plans[0].core, CoreKind::Xray);
        assert_eq!(plans[1].core, CoreKind::SingBox);
        assert_eq!(plans[1].config["outbounds"][0]["type"], "socks");

        // "xray" is impossible for hysteria2: warn and keep the native sing-box outbound.
        let hy2 = server(
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
        let plans = plan(&hy2, &tun("xray"));
        assert_eq!(plans.len(), 1);
        assert_eq!(plans[0].config["outbounds"][0]["type"], "hysteria2");

        // "sing-box" is impossible for the xhttp chain: warn and keep the Xray hop.
        let plans = plan(&vless_xhttp(), &tun("sing-box"));
        assert_eq!(plans.len(), 2);
        assert_eq!(plans[0].core, CoreKind::Xray);
        assert_eq!(plans[1].config["outbounds"][0]["type"], "socks");
    }

    #[test]
    fn tun_internal_xray_hop_has_no_user_auth() {
        // A TUN + xhttp (Xray-only transport) + user local auth: the internal Xray hop must
        // NOT require credentials, because sing-box dials it without any.
        let settings = Settings {
            mode: ConnectionMode::Tun,
            local_auth_user: "user".into(),
            local_auth_pass: "pass".into(),
            ..Settings::default()
        };
        let plans = plan(&vless_xhttp(), &settings);
        assert_eq!(plans.len(), 2);
        assert_eq!(plans[0].core, CoreKind::Xray);
        let socks_in = &plans[0].config["inbounds"][0];
        assert_eq!(socks_in["tag"], "socks");
        assert_eq!(socks_in["settings"]["auth"], "noauth");
        assert!(socks_in["settings"].get("accounts").is_none());

        // TUN mode has no user-facing mixed inbound (the TUN is the boundary), so the user
        // credentials never leak into the sing-box config either.
        assert!(plans[1].config["inbounds"]
            .as_array()
            .unwrap()
            .iter()
            .all(|inbound| inbound["type"] != json!("mixed")));
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
        let mut settings = Settings {
            mode: ConnectionMode::Tun,
            ..Settings::default()
        };
        // Global: the RF preset closes with its own conditionless direct catch-all.
        settings.routing.preset = crate::model::RoutingPreset::Global;
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
        settings.failover.probe_interval_ms = 15000;
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
        assert_eq!(cfg["observatory"]["probeInterval"], json!("15000ms"));
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
        // The RF preset falls back to DIRECT; this test is about the `proxy` urltest.
        settings.routing.preset = crate::model::RoutingPreset::Global;
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
        assert_eq!(outbounds[0]["interval"], json!("15000ms"));
        assert_eq!(outbounds[0]["tolerance"], json!(80));
        assert_eq!(outbounds[1]["tag"], json!("node-0"));
        assert_eq!(outbounds[2]["tag"], json!("node-1"));
        // route.final and DNS detour keep pointing at `proxy`.
        assert_eq!(cfg["route"]["final"], json!("proxy"));
        assert_eq!(cfg["dns"]["servers"][0]["detour"], json!("proxy"));
    }

    #[test]
    fn plan_cluster_drops_incompatible_backups_xray() {
        // Xray primary + Xray backup + a sing-box-only backup (hysteria2).
        let nodes = [vless_xhttp(), trojan_ws(), hy2()];
        let settings = failover_settings();
        let plans = plan_cluster(&nodes, &settings, &ConnectContext::default());
        assert_eq!(plans.len(), 1);
        assert_eq!(plans[0].core, CoreKind::Xray);
        let cfg = &plans[0].config;
        let outbounds = cfg["outbounds"].as_array().unwrap();
        let tags: Vec<&str> = outbounds.iter().filter_map(|o| o["tag"].as_str()).collect();
        // hysteria2 dropped: only the two Xray-capable nodes remain.
        assert_eq!(tags, ["node-0", "node-1", "direct", "block"]);
        assert_eq!(
            cfg["routing"]["balancers"][0]["selector"],
            json!(["node-0", "node-1"])
        );
        // No proxy-capable core would ever leak via a `freedom` node (the `direct` outbound
        // is legitimately `freedom`).
        let is_direct = |o: &serde_json::Value| o["tag"] == json!("direct");
        assert!(
            !outbounds
                .iter()
                .any(|o| o["protocol"] == json!("freedom") && !is_direct(o)),
            "cluster must not contain a leaked direct/freedom node"
        );

        // Defensive: the generator itself never emits a freedom backup either.
        let cfg = xray::full_config_cluster_with(
            &[vless_xhttp(), hy2()],
            &settings,
            10818,
            &ConnectContext::default(),
        );
        let outbounds = cfg["outbounds"].as_array().unwrap();
        assert!(!outbounds
            .iter()
            .any(|o| o["protocol"] == json!("freedom") && !is_direct(o)));
        // Only the primary survives -> single-node config, no balancer.
        assert!(cfg["routing"].get("balancers").is_none());
        assert_eq!(cfg["outbounds"][0]["tag"], json!("proxy"));
    }

    #[test]
    fn plan_cluster_singbox_only_primary_drops_incompatible_backup() {
        // sing-box-only primary (hysteria2) + an xhttp backup sing-box cannot express.
        let nodes = [hy2(), vless_xhttp()];
        let settings = failover_settings();
        let plans = plan_cluster(&nodes, &settings, &ConnectContext::default());
        assert_eq!(plans.len(), 1);
        assert_eq!(plans[0].core, CoreKind::SingBox);
        let outbounds = plans[0].config["outbounds"].as_array().unwrap();
        // xhttp backup dropped, so no urltest: the plain single `proxy` outbound.
        assert_eq!(outbounds[0]["tag"], json!("proxy"));
        assert_eq!(outbounds[0]["type"], json!("hysteria2"));

        // A sing-box-capable backup (trojan/ws) is kept and gets the urltest balancer.
        let nodes = [hy2(), trojan_ws()];
        let cfg = &plan_cluster(&nodes, &settings, &ConnectContext::default())[0].config;
        let outbounds = cfg["outbounds"].as_array().unwrap();
        assert_eq!(outbounds[0]["type"], json!("urltest"));
        assert_eq!(outbounds[0]["outbounds"], json!(["node-0", "node-1"]));
        assert_eq!(outbounds[1]["type"], json!("hysteria2"));
        assert_eq!(outbounds[2]["type"], json!("trojan"));
    }

    #[test]
    fn failover_probe_fallbacks_are_non_empty() {
        let nodes = [vless_xhttp(), trojan_ws()];
        let mut settings = failover_settings();
        settings.ping_url = String::new();
        settings.failover.probe_url = "   ".into();
        settings.failover.probe_interval_ms = 0;

        // Xray: constant probe URL + clamped interval.
        let cfg = &plan_cluster(&nodes, &settings, &ConnectContext::default())[0].config;
        assert_eq!(cfg["observatory"]["probeUrl"], json!(DEFAULT_PROBE_URL));
        assert_eq!(cfg["observatory"]["probeInterval"], json!("15000ms"));

        // sing-box: same fallbacks.
        let nodes = [hy2(), trojan_ws()];
        let cfg = &plan_cluster(&nodes, &settings, &ConnectContext::default())[0].config;
        assert_eq!(cfg["outbounds"][0]["url"], json!(DEFAULT_PROBE_URL));
        assert_eq!(cfg["outbounds"][0]["interval"], json!("15000ms"));

        // A tiny interval is clamped up too.
        settings.failover.probe_interval_ms = 1;
        let cfg = &plan_cluster(&nodes, &settings, &ConnectContext::default())[0].config;
        assert_eq!(cfg["outbounds"][0]["interval"], json!("15000ms"));
    }
}

/// Exhaustive validation of generated configs against the real core binaries.
///
/// The binaries are the oracle: `FLAMIX_XRAY_BIN` / `FLAMIX_SINGBOX_BIN` / `FLAMIX_MIHOMO_BIN`
/// (mihomo also reads geo databases from `FLAMIX_GEO_DIR`, default its own directory). When unset
/// the tests return early (so normal `cargo test` stays hermetic). Every protocol
/// sample x {Proxy, TUN} x {Global, BypassRu, Custom(each rule kind)} is validated,
/// plus the TUN xhttp chain. Any non-zero exit *or* deprecation warning fails with
/// the config name and the core's own message.
#[cfg(test)]
mod validation {
    use super::*;
    use crate::model::{
        Credentials, Dns, FailoverMode, MirrorMode, Network, Routing, RoutingPreset, RoutingRule,
        RuleAction, Security, Transport,
    };
    use serde_json::json;
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
            anixart_mirror: MirrorMode::default(),
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

    /// Every preset x Anixart mirror Direct/Proxy.
    fn presets() -> Vec<(&'static str, Routing)> {
        let mut out = Vec::new();
        for (mname, mirror) in [
            ("mirror-direct", MirrorMode::Direct),
            ("mirror-proxy", MirrorMode::Proxy),
        ] {
            let named: [(&str, Routing); 3] = [
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
            ];
            for (pname, routing) in named {
                let name: &'static str = Box::leak(format!("{pname}/{mname}").into_boxed_str());
                out.push((
                    name,
                    Routing {
                        anixart_mirror: mirror,
                        ..routing
                    },
                ));
            }
        }
        out
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
            CoreKind::Mihomo => std::env::var("FLAMIX_MIHOMO_BIN"),
        };
        let bin = bin.ok()?;
        let mut command = std::process::Command::new(&bin);
        let file = if core == CoreKind::Mihomo {
            // mihomo reads YAML; give it a private home dir that already has the geo
            // databases, so `-t` never has to download them.
            let home = mihomo_home(std::path::Path::new(&bin));
            command.arg("-t").arg("-d").arg(home).arg("-f");
            let yaml = mihomo::render_yaml(cfg).unwrap();
            let file = write_temp(name, &yaml);
            let yaml_path = file.with_extension("yaml");
            std::fs::rename(&file, &yaml_path).unwrap();
            yaml_path
        } else {
            let file = write_temp(name, &serde_json::to_string(cfg).unwrap());
            match core {
                CoreKind::Xray => command.args(["run", "-test", "-c"]),
                _ => command.args(["check", "-c"]),
            };
            file
        };
        let out = command.arg(&file).output().unwrap();
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
        if is_deprecation(&message) || (core == CoreKind::Mihomo && mihomo_warns(&message)) {
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

    /// mihomo logs `level=warning` / `level=error` lines for anything it dislikes.
    fn mihomo_warns(message: &str) -> bool {
        message.lines().any(|l| {
            l.contains("level=warning") || l.contains("level=error") || l.contains("level=fatal")
        })
    }

    /// A per-process mihomo home dir with `geosite.dat` / `geoip.dat` linked from
    /// `FLAMIX_GEO_DIR` (default: the directory of the mihomo binary).
    fn mihomo_home(bin: &std::path::Path) -> std::path::PathBuf {
        static HOME: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();
        HOME.get_or_init(|| {
            let home = std::env::temp_dir().join(format!("flamix-mihomo-{}", std::process::id()));
            std::fs::create_dir_all(&home).unwrap();
            let geo = std::env::var_os("FLAMIX_GEO_DIR")
                .map(std::path::PathBuf::from)
                .or_else(|| bin.parent().map(std::path::Path::to_path_buf))
                .unwrap();
            for name in ["geosite.dat", "geoip.dat"] {
                let _ = std::fs::remove_file(home.join(name));
                std::fs::copy(geo.join(name), home.join(name)).unwrap();
            }
            home
        })
        .clone()
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
                    let api = api_port(CoreKind::Mihomo, &settings);
                    if let Some(cfg) = mihomo::full_config(&srv, &settings, api) {
                        problems.extend(check_core(
                            CoreKind::Mihomo,
                            &cfg,
                            &format!("{sname}/{pname}/{mode_name}/mihomo"),
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
        let mut problems = Vec::new();
        // Routing variants: no provider, and both provider profiles (incl. global-direct).
        let providers = provider_routings();
        let mut views: Vec<(&str, Option<&ProviderRouting>)> = vec![("no-provider", None)];
        for (name, provider) in &providers {
            views.push((name, Some(provider)));
        }
        for (label, mode) in [
            ("failover", FailoverMode::Failover),
            ("load-balance", FailoverMode::LoadBalance),
        ] {
            for (pname, provider) in &views {
                let ctx = ConnectContext {
                    provider: *provider,
                    overrides: None,
                };
                let mut settings = settings(ConnectionMode::Proxy, Routing::default());
                settings.failover.enabled = true;
                settings.failover.mode = mode;
                let api = api_port(CoreKind::Xray, &settings);
                let cfg = xray::full_config_cluster_with(&nodes, &settings, api, &ctx);
                // Ordering assertions: proxy rules use the balancer; the final catch-all
                // is the balancer unless the provider goes direct.
                let rules = cfg["routing"]["rules"].as_array().unwrap();
                // No provider: the default RF preset closes with a direct catch-all.
                let direct_final = provider.map_or(routing::final_direct(&settings.routing), |p| {
                    !p.global_proxy
                });
                if direct_final {
                    let last = rules.last().unwrap();
                    assert_eq!(last["outboundTag"], json!("direct"), "{pname}/{label}");
                    assert!(last.get("balancerTag").is_none(), "{pname}/{label}");
                } else {
                    let last = rules.last().unwrap();
                    assert_eq!(last["balancerTag"], json!("proxy"), "{pname}/{label}");
                }
                assert!(
                    !rules.iter().any(|r| r["outboundTag"] == json!("proxy")),
                    "{pname}/{label}: no rule may target a non-existent proxy outbound"
                );
                problems.extend(check_core(
                    CoreKind::Xray,
                    &cfg,
                    &format!("multi-node/{label}/{pname}/xray"),
                ));
                let api = api_port(CoreKind::SingBox, &settings);
                let cfg =
                    singbox::full_config_cluster_with(&nodes, &settings, false, None, api, &ctx);
                problems.extend(check_core(
                    CoreKind::SingBox,
                    &cfg,
                    &format!("multi-node/{label}/{pname}/sing-box"),
                ));
            }
        }
        // Failover off: byte-for-byte the single-server config.
        let ctx = ConnectContext::default();
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
                ..Dns::default()
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
                ..Dns::default()
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

    /// Every settings-overhaul knob (inbound, TUN, DNS, mux) against the real cores.
    #[test]
    fn settings_overhaul_knobs_are_accepted_by_the_real_cores() {
        if std::env::var("FLAMIX_XRAY_BIN").is_err() && std::env::var("FLAMIX_SINGBOX_BIN").is_err()
        {
            return;
        }
        let Some((_, srv)) = samples().into_iter().find(|(n, _)| *n == "vless-ws") else {
            return;
        };
        let mut problems = Vec::new();

        // Xray proxy: UDP off, sniffing off, user auth, mux concurrency, DNS knobs.
        let settings = Settings {
            mode: ConnectionMode::Proxy,
            udp_enabled: false,
            sniffing: false,
            mux: true,
            mux_concurrency: 16,
            local_auth_user: "user".into(),
            local_auth_pass: "pass".into(),
            dns: Dns {
                parallel: true,
                serve_stale: true,
                hosts: BTreeMap::from([("host.example".into(), "10.0.0.7".into())]),
                ..Dns::default()
            },
            ..Settings::default()
        };
        let api = api_port(CoreKind::Xray, &settings);
        problems.extend(check_core(
            CoreKind::Xray,
            &xray::full_config(&srv, &settings, api),
            "overhaul/proxy/xray",
        ));

        // sing-box proxy: mux on.
        let api = api_port(CoreKind::SingBox, &settings);
        problems.extend(check_core(
            CoreKind::SingBox,
            &singbox::full_config(&srv, &settings, false, None, api),
            "overhaul/proxy/sing-box",
        ));

        // sing-box TUN: every ICMP mode with auto-route off, strict-route on, custom exclude,
        // fake-ip range, bootstrap, SVCB blocking and static hosts.
        for icmp in ["rule", "direct", "unreachable", "drop", "reply"] {
            let settings = Settings {
                mode: ConnectionMode::Tun,
                tun_auto_route: false,
                tun_strict_route: true,
                tun_route_exclude: vec!["203.0.113.0/24".into()],
                tun_icmp: icmp.into(),
                dns: Dns {
                    fake_ip: true,
                    fake_ip_range: "198.19.0.0/16".into(),
                    bootstrap: "77.88.8.8".into(),
                    block_svcb: true,
                    hosts: BTreeMap::from([("host.example".into(), "10.0.0.7".into())]),
                    ..Dns::default()
                },
                ..Settings::default()
            };
            let api = api_port(CoreKind::SingBox, &settings);
            let cfg = singbox::full_config(&srv, &settings, true, None, api);
            problems.extend(check_core(
                CoreKind::SingBox,
                &cfg,
                &format!("overhaul/tun/{icmp}/sing-box"),
            ));
        }
        report(problems);
    }

    /// TUN + an Xray-only transport + user local auth: both cores must accept the chain, and
    /// the internal Xray hop must not require credentials.
    #[test]
    fn tun_user_auth_internal_hop_is_accepted_by_the_real_cores() {
        if std::env::var("FLAMIX_XRAY_BIN").is_err() && std::env::var("FLAMIX_SINGBOX_BIN").is_err()
        {
            return;
        }
        let Some((_, srv)) = samples().into_iter().find(|(n, _)| *n == "vless-xhttp") else {
            return;
        };
        let settings = Settings {
            mode: ConnectionMode::Tun,
            local_auth_user: "user".into(),
            local_auth_pass: "pass".into(),
            ..Settings::default()
        };
        let plans = super::plan(&srv, &settings);
        assert_eq!(plans.len(), 2, "xhttp chain must be xray + sing-box");
        assert_eq!(plans[0].core, CoreKind::Xray);
        // The internal hop carries no auth (sing-box dials it without credentials).
        assert_eq!(plans[0].config["inbounds"][0]["settings"]["auth"], "noauth");
        let mut problems = Vec::new();
        for plan in &plans {
            problems.extend(check_core(
                plan.core,
                &plan.config,
                &format!("tun-user-auth/{}", plan.core.binary_name()),
            ));
        }
        report(problems);
    }
}

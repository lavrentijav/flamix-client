//! sing-box config for the Android `VpnService` mode (milestone M3).
//!
//! Data path: the TUN fd from `VpnService.Builder.establish()` is read by tun2proxy inside the app
//! (`app/flamix-android/src/tunnel.rs`) and forwarded to a local SOCKS5 inbound of sing-box. The
//! stock sing-box binary cannot take a TUN fd (`file_descriptor` is not a config option; it is
//! only injected by libbox), so the core runs in the plain proxy shape: the regular
//! [`crate::config::singbox`] config with a `mixed` inbound on `127.0.0.1`, same proxy outbound,
//! DNS and routing. Per-app exclusion (the app itself is not tunnelled) is done by the VPN builder.

use crate::config::singbox;
use crate::config::ConnectContext;
use crate::model::{Server, Settings};
use serde_json::Value;

/// MTU of the `VpnService` interface (must equal `Builder.setMtu`).
pub const ANDROID_TUN_MTU: u32 = 1500;
/// Interface address (must equal `Builder.addAddress("172.19.0.1", 30)`).
pub const ANDROID_TUN_ADDRESS: &str = "172.19.0.1/30";

/// Builds the sing-box config the Android VPN service runs.
///
/// Compared to the desktop proxy config: the `mixed` inbound is forced to `127.0.0.1` (never LAN)
/// and has no `users` (tun2proxy connects without credentials; the port is loopback-only).
/// Returns `None` when the server has no sing-box outbound (e.g. xhttp, which needs Xray).
pub fn android_config(
    server: &Server,
    backups: &[&Server],
    settings: &Settings,
    api_port: u16,
    ctx: &ConnectContext,
) -> Option<Value> {
    singbox::outbound(server, settings)?;
    // Backups (chosen by the caller, by exit IP) form a failover cluster only when enabled.
    let nodes: Vec<Server> = std::iter::once(server)
        .chain(backups.iter().copied())
        .cloned()
        .collect();
    let mut cfg = if settings.failover.enabled && nodes.len() > 1 {
        singbox::full_config_cluster_with(&nodes, settings, false, None, api_port, ctx)
    } else {
        singbox::full_config_with(server, settings, false, None, api_port, ctx)
    };
    // Stock sing-box on Android (non-root) dies at start with "netlink socket in Android is
    // banned by Google" when the network monitor is created, which `auto_detect_interface` needs.
    // Not needed here: the app is excluded from the VPN, so the core's sockets use the real network.
    if let Some(route) = cfg.get_mut("route").and_then(Value::as_object_mut) {
        route.remove("auto_detect_interface");
    }
    strip_remote_rule_sets(&mut cfg);
    // Core output is copied into flamix.log (Settings -> Логи); warn keeps it readable.
    cfg["log"] = serde_json::json!({"level": "warn", "timestamp": false});
    for inbound in cfg.get_mut("inbounds")?.as_array_mut()? {
        if inbound["type"] == "mixed" {
            let ib = inbound.as_object_mut()?;
            ib.insert("listen".into(), "127.0.0.1".into());
            ib.remove("users");
        }
    }
    Some(cfg)
}

/// Removes remote `.srs` rule sets and every route/DNS rule that uses them. The core downloads
/// them at start (through the proxy); if the download fails or is slow the core exits with FATAL
/// and the VPN drops at once. TODO: ship the rule sets as local files and re-enable routing.
fn strip_remote_rule_sets(cfg: &mut Value) {
    if let Some(route) = cfg.get_mut("route").and_then(Value::as_object_mut) {
        route.remove("rule_set");
    }
    for (section, key) in [("route", "rules"), ("dns", "rules")] {
        if let Some(rules) = cfg
            .get_mut(section)
            .and_then(|s| s.get_mut(key))
            .and_then(Value::as_array_mut)
        {
            rules.retain(|r| !r.to_string().contains("\"rule_set\""));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Credentials, Network, Security, Transport};
    use serde_json::json;

    fn vless() -> Server {
        Server {
            id: "t".into(),
            name: "t".into(),
            country: None,
            address: "example.com".into(),
            port: 443,
            credentials: Credentials::Vless {
                uuid: "11111111-1111-1111-1111-111111111111".into(),
                flow: None,
                encryption: None,
            },
            transport: Transport::default(),
            security: Security::default(),
            raw_link: None,
            traffic: None,
            local_traffic: Default::default(),
            last_ping_ms: None,
        }
    }

    fn cfg(settings: &Settings) -> Value {
        android_config(&vless(), &[], settings, 9090, &ConnectContext::default()).unwrap()
    }

    #[test]
    fn android_config_has_no_auto_detect_interface() {
        let c = cfg(&Settings::default());
        assert!(c["route"].get("auto_detect_interface").is_none());
        assert!(c["route"].get("default_interface").is_none());
    }

    #[test]
    fn remote_rule_sets_are_stripped() {
        let mut c = json!({
            "route": {"rule_set": [{"tag": "x"}], "rules": [{"rule_set": ["x"], "outbound": "direct"}, {"protocol": "dns"}]},
            "dns": {"rules": [{"rule_set": ["x"], "server": "d"}]}
        });
        strip_remote_rule_sets(&mut c);
        assert!(c["route"].get("rule_set").is_none());
        assert_eq!(c["route"]["rules"].as_array().unwrap().len(), 1);
        assert!(c["dns"]["rules"].as_array().unwrap().is_empty());
    }

    #[test]
    fn android_inbound_is_loopback_mixed_without_auth() {
        let s = Settings {
            allow_lan: true,
            local_auth_user: "u".into(),
            local_auth_pass: "p".into(),
            ..Settings::default()
        };
        let c = cfg(&s);
        let inbounds = c["inbounds"].as_array().unwrap();
        assert_eq!(inbounds.len(), 1);
        assert_eq!(inbounds[0]["type"], json!("mixed"));
        assert_eq!(inbounds[0]["listen"], json!("127.0.0.1"));
        assert_eq!(inbounds[0]["listen_port"], json!(s.socks_port));
        assert!(inbounds[0].get("users").is_none());
    }

    #[test]
    fn android_config_reuses_proxy_outbound_and_dns() {
        let c = cfg(&Settings::default());
        let proxy = c["outbounds"]
            .as_array()
            .unwrap()
            .iter()
            .find(|o| o["tag"] == json!("proxy"))
            .unwrap();
        assert_eq!(
            *proxy,
            singbox::outbound(&vless(), &Settings::default()).unwrap()
        );
        assert!(c["dns"]["servers"]
            .as_array()
            .is_some_and(|s| !s.is_empty()));
        let rules = c["route"]["rules"].as_array().unwrap();
        assert!(rules.iter().any(|r| r["action"] == json!("hijack-dns")));
    }

    #[test]
    fn android_config_none_for_non_singbox_server() {
        let mut s = vless();
        s.transport.network = Network::Xhttp;
        assert!(android_config(
            &s,
            &[],
            &Settings::default(),
            9090,
            &ConnectContext::default()
        )
        .is_none());
    }

    /// `sing-box check` on the generated config (real binary via FLAMIX_SINGBOX_BIN).
    #[test]
    #[ignore = "needs FLAMIX_SINGBOX_BIN"]
    fn android_config_passes_singbox_check() {
        let Ok(bin) = std::env::var("FLAMIX_SINGBOX_BIN") else {
            return;
        };
        let path = std::env::temp_dir().join(format!("flamix-android-{}.json", std::process::id()));
        std::fs::write(
            &path,
            serde_json::to_string(&cfg(&Settings::default())).unwrap(),
        )
        .unwrap();
        let out = std::process::Command::new(bin)
            .args(["check", "-c"])
            .arg(&path)
            .output()
            .unwrap();
        let _ = std::fs::remove_file(&path);
        let msg = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(out.status.success(), "sing-box check failed:\n{msg}");
        assert!(!msg.to_lowercase().contains("deprecat"), "{msg}");
    }
}

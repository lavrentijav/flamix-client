//! Linux transparent proxy — install/remove the nftables TPROXY ruleset and the
//! policy-routing bits that make it work.
//!
//! The ruleset lives in one dedicated `inet` table (`flamix`) so uninstall is a single
//! `nft delete table` plus two `ip` calls. The caller runs these as root (same elevated
//! watchdog that owns the sing-box core).
//!
//! See `docs/transparent-proxy.md` for the design.

use flamix_core::model::Settings;

/// Table name owned by this module. `delete table inet flamix` is the full uninstall.
pub const TABLE: &str = "flamix";

/// Baked-in bypass CIDRs — private, loopback, link-local, multicast, broadcast, CGNAT.
/// Any packet to these ranges must leave through the real route, never TPROXY, or the
/// local subnet (and the connection to the proxy server itself) becomes unreachable.
const PRIVATE_V4: &[&str] = &[
    "0.0.0.0/8",
    "10.0.0.0/8",
    "100.64.0.0/10",
    "127.0.0.0/8",
    "169.254.0.0/16",
    "172.16.0.0/12",
    "192.0.0.0/24",
    "192.168.0.0/16",
    "224.0.0.0/4",
    "240.0.0.0/4",
    "255.255.255.255/32",
];

/// Everything that `install` / `remove` need from the user settings.
#[derive(Debug, Clone)]
pub struct TproxyPlan {
    /// Local port the sing-box tproxy inbound listens on.
    pub port: u16,
    /// fwmark used for policy routing of locally-generated traffic.
    pub mark: u32,
    /// Routing table id the fwmark looks up.
    pub table: u32,
    /// uid of the process that runs the core (its egress is bypassed to avoid loops).
    pub core_uid: u32,
    /// Extra v4 CIDRs that must bypass the tunnel on top of [`PRIVATE_V4`].
    pub extra_v4_bypass: Vec<String>,
    /// Access-point interfaces whose forwarded traffic is TPROXY'd into the core.
    /// Empty vector = AP tether is off.
    pub ap_interfaces: Vec<String>,
}

impl TproxyPlan {
    /// Build a plan from `Settings` + the uid under which the elevated core will run.
    pub fn from_settings(settings: &Settings, core_uid: u32) -> Self {
        let ap_interfaces = if settings.ap_tether_enabled {
            settings
                .ap_interfaces
                .iter()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        } else {
            Vec::new()
        };
        let extra_v4_bypass = settings
            .tproxy_route_exclude
            .iter()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        Self {
            port: settings.tproxy_port,
            mark: settings.tproxy_mark,
            table: settings.tproxy_table,
            core_uid,
            extra_v4_bypass,
            ap_interfaces,
        }
    }
}

/// Render the nftables ruleset for [`TproxyPlan`]. Fed to `nft -f -` by [`install`].
///
/// Pure, deterministic — this is what the unit tests check.
pub fn build_nft(plan: &TproxyPlan) -> String {
    let mark_hex = format!("0x{:x}", plan.mark);
    let mut bypass: Vec<String> = PRIVATE_V4.iter().map(|s| s.to_string()).collect();
    for cidr in &plan.extra_v4_bypass {
        if !bypass.iter().any(|c| c == cidr) {
            bypass.push(cidr.clone());
        }
    }
    let bypass_elems = bypass.join(", ");
    let ap_elems: String = plan.ap_interfaces.join(", ");

    let mut out = String::new();
    // Clean install: delete any leftover table, ignore errors (first run has nothing to delete).
    out.push_str(&format!("add table inet {TABLE}\n"));
    out.push_str(&format!("delete table inet {TABLE}\n"));
    out.push_str(&format!("add table inet {TABLE}\n"));

    // Bypass set — static for this install.
    out.push_str(&format!(
        "add set inet {TABLE} bypass_v4 {{ type ipv4_addr; flags interval; elements = {{ {bypass_elems} }}; }}\n"
    ));
    if !plan.ap_interfaces.is_empty() {
        out.push_str(&format!(
            "add set inet {TABLE} ap_ifaces {{ type ifname; elements = {{ {ap_elems} }}; }}\n"
        ));
    }

    // divert: match existing TPROXY-origin sockets so their reply packets get the mark instead
    // of being rerouted again. nftables `socket transparent 1` matches a listening transparent
    // socket for the 4-tuple.
    out.push_str(&format!(
        "add chain inet {TABLE} divert {{ type filter hook prerouting priority mangle; policy accept; }}\n"
    ));
    out.push_str(&format!(
        "add rule inet {TABLE} divert meta l4proto {{ tcp, udp }} socket transparent 1 meta mark set {mark_hex} accept\n"
    ));

    // prerouting (AP tether path): forwarded TCP/UDP from AP interfaces → TPROXY into core.
    out.push_str(&format!(
        "add chain inet {TABLE} prerouting {{ type filter hook prerouting priority mangle; policy accept; }}\n"
    ));
    if !plan.ap_interfaces.is_empty() {
        // Bypass private destinations so AP clients still reach the LAN / router.
        out.push_str(&format!(
            "add rule inet {TABLE} prerouting iifname @ap_ifaces ip daddr @bypass_v4 accept\n"
        ));
        out.push_str(&format!(
            "add rule inet {TABLE} prerouting iifname @ap_ifaces meta l4proto tcp tproxy ip to :{port} meta mark set {mark_hex} accept\n",
            port = plan.port
        ));
        out.push_str(&format!(
            "add rule inet {TABLE} prerouting iifname @ap_ifaces meta l4proto udp tproxy ip to :{port} meta mark set {mark_hex} accept\n",
            port = plan.port
        ));
    }

    // output (locally-generated traffic): mark it so `ip rule fwmark ... lookup <table>` sends
    // it back through lo, where the core's transparent socket picks it up. Loop prevention is
    // by skuid: packets originating from the core's own uid never match.
    out.push_str(&format!(
        "add chain inet {TABLE} output {{ type route hook output priority mangle; policy accept; }}\n"
    ));
    out.push_str(&format!(
        "add rule inet {TABLE} output meta skuid {uid} accept\n",
        uid = plan.core_uid
    ));
    out.push_str(&format!(
        "add rule inet {TABLE} output ip daddr @bypass_v4 accept\n"
    ));
    out.push_str(&format!(
        "add rule inet {TABLE} output meta l4proto {{ tcp, udp }} meta mark set {mark_hex}\n"
    ));
    out
}

/// Commands used by the elevated watchdog to make the kernel route marked packets to `lo`.
///
/// The watchdog runs these as root right after `nft -f -` and before launching the core.
pub fn ip_rule_add(plan: &TproxyPlan) -> [String; 2] {
    [
        format!(
            "ip -4 rule add fwmark {mark}/{mark} lookup {table} priority 100",
            mark = plan.mark,
            table = plan.table,
        ),
        format!(
            "ip -4 route add local 0.0.0.0/0 dev lo table {}",
            plan.table
        ),
    ]
}

/// Teardown counterparts of [`ip_rule_add`]. Each line is best-effort; a missing rule means
/// nothing to delete, not an error.
pub fn ip_rule_del(plan: &TproxyPlan) -> [String; 2] {
    [
        format!(
            "ip -4 rule del fwmark {mark}/{mark} lookup {table} priority 100",
            mark = plan.mark,
            table = plan.table,
        ),
        format!(
            "ip -4 route del local 0.0.0.0/0 dev lo table {}",
            plan.table
        ),
    ]
}

/// Full teardown input for `nft -f -`.
pub fn nft_delete() -> String {
    format!("delete table inet {TABLE}\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use flamix_core::model::ConnectionMode;

    fn settings_tproxy() -> Settings {
        Settings {
            mode: ConnectionMode::Tproxy,
            tproxy_port: 12345,
            tproxy_mark: 0x1a2b,
            tproxy_table: 100,
            ..Settings::default()
        }
    }

    #[test]
    fn plan_from_settings_filters_blanks_and_disables_ap_when_off() {
        let mut s = settings_tproxy();
        s.tproxy_route_exclude = vec!["203.0.113.0/24".into(), "  ".into()];
        s.ap_interfaces = vec!["wlan0".into()];
        s.ap_tether_enabled = false;
        let plan = TproxyPlan::from_settings(&s, 999);
        assert_eq!(plan.port, 12345);
        assert_eq!(plan.extra_v4_bypass, vec!["203.0.113.0/24".to_string()]);
        assert!(plan.ap_interfaces.is_empty(), "AP disabled -> empty list");

        s.ap_tether_enabled = true;
        let plan = TproxyPlan::from_settings(&s, 999);
        assert_eq!(plan.ap_interfaces, vec!["wlan0".to_string()]);
    }

    #[test]
    fn nft_contains_divert_output_and_bypass() {
        let plan = TproxyPlan::from_settings(&settings_tproxy(), 42);
        let nft = build_nft(&plan);

        assert!(nft.contains("add table inet flamix"));
        assert!(
            nft.contains("delete table inet flamix"),
            "install flushes a stale leftover table before adding its own"
        );
        assert!(nft.contains("flags interval"));
        assert!(nft.contains("192.168.0.0/16"), "RFC1918 is baked in");
        assert!(nft.contains("127.0.0.0/8"), "loopback is baked in");
        assert!(nft.contains("169.254.0.0/16"), "link-local is baked in");
        assert!(nft.contains("224.0.0.0/4"), "multicast is baked in");
        assert!(nft.contains("100.64.0.0/10"), "CGNAT is baked in");

        assert!(
            nft.contains("socket transparent 1 meta mark set 0x1a2b"),
            "divert chain marks already-established transparent sockets"
        );
        assert!(
            nft.contains("meta skuid 42 accept"),
            "output chain skips the core's uid to prevent loops"
        );
        assert!(
            nft.contains("meta mark set 0x1a2b"),
            "output chain marks the rest"
        );
    }

    #[test]
    fn ap_rules_only_when_interfaces_set() {
        let mut s = settings_tproxy();
        s.ap_tether_enabled = true;
        s.ap_interfaces = vec!["wlan0".into(), "usb0".into()];
        let plan = TproxyPlan::from_settings(&s, 42);
        let nft = build_nft(&plan);
        assert!(nft.contains("elements = { wlan0, usb0 }"));
        assert!(nft.contains("iifname @ap_ifaces meta l4proto tcp tproxy ip to :12345"));
        assert!(nft.contains("iifname @ap_ifaces meta l4proto udp tproxy ip to :12345"));

        // AP off → no ap_ifaces set, no AP rules.
        let mut s = settings_tproxy();
        s.ap_tether_enabled = false;
        s.ap_interfaces = vec!["wlan0".into()];
        let plan = TproxyPlan::from_settings(&s, 42);
        let nft = build_nft(&plan);
        assert!(!nft.contains("ap_ifaces"));
        assert!(!nft.contains("tproxy ip to"));
    }

    #[test]
    fn user_bypass_merges_with_baked_in() {
        let mut s = settings_tproxy();
        s.tproxy_route_exclude = vec!["203.0.113.0/24".into(), "192.168.0.0/16".into()];
        let plan = TproxyPlan::from_settings(&s, 42);
        let nft = build_nft(&plan);
        // Baked-in duplicate is kept once; the extra range is appended.
        let count = nft.matches("192.168.0.0/16").count();
        assert_eq!(count, 1, "no duplicate for an already-baked-in CIDR");
        assert!(nft.contains("203.0.113.0/24"));
    }

    #[test]
    fn ip_rule_strings_use_fwmark_and_table() {
        let plan = TproxyPlan::from_settings(&settings_tproxy(), 42);
        let [rule, route] = ip_rule_add(&plan);
        assert_eq!(
            rule,
            "ip -4 rule add fwmark 6699/6699 lookup 100 priority 100"
        );
        assert_eq!(route, "ip -4 route add local 0.0.0.0/0 dev lo table 100");
        let [rule_del, route_del] = ip_rule_del(&plan);
        assert!(rule_del.starts_with("ip -4 rule del fwmark"));
        assert!(route_del.starts_with("ip -4 route del local 0.0.0.0/0 dev lo"));
    }

    #[test]
    fn nft_delete_removes_the_table() {
        assert_eq!(nft_delete(), "delete table inet flamix\n");
    }
}

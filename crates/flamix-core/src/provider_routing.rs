//! Provider-pushed routing profiles.
//!
//! Providers deliver routing in several shapes (see `docs/research/happ.md`,
//! `docs/research/v2raytun.md`); this module normalises all of them into
//! [`ProviderRouting`] / [`RoutingRule`] (the Flamix model):
//!
//! - Happ routing profile JSON (`routing` header body / `happ://routing/{add,onadd}/<base64>`);
//! - v2RayTun `routing` header: base64 of an Xray-style routing JSON (`rules[]` + `outboundTag`);
//! - the `happ://routing/...` deep link (`add`, `onadd`, `off`).

use crate::model::{Dns, ProviderRouting, RoutingRule, RuleAction};
use crate::{Error, Result};
use serde_json::Value;
use std::collections::BTreeMap;

/// A `happ://routing/...` link, decoded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RoutingLinkAction {
    /// Add the profile (`happ://routing/add/<b64>`).
    Add(ProviderRouting),
    /// Add and activate the profile (`happ://routing/onadd/<b64>`).
    AddAndActivate(ProviderRouting),
    /// Disable provider routing (`happ://routing/off`).
    Off,
}

/// Parse a `happ://routing/...` link. Tolerates std/url-safe base64, a UTF-8 BOM and
/// case-insensitive JSON keys.
pub fn from_happ_link(link: &str) -> Result<RoutingLinkAction> {
    let rest = strip_happ_routing(link)
        .ok_or_else(|| Error::Parse(format!("not a happ routing link: {link}")))?;
    let (action, payload) = rest.split_once('/').unwrap_or((rest, ""));
    let action = action.trim();
    if action.eq_ignore_ascii_case("off") {
        return Ok(RoutingLinkAction::Off);
    }
    if !action.eq_ignore_ascii_case("add") && !action.eq_ignore_ascii_case("onadd") {
        return Err(Error::Parse(format!(
            "unknown happ routing action: {action}"
        )));
    }
    let json = decode_profile_json(payload)?;
    let routing = from_happ_profile(&json)?;
    if action.eq_ignore_ascii_case("onadd") {
        Ok(RoutingLinkAction::AddAndActivate(routing))
    } else {
        Ok(RoutingLinkAction::Add(routing))
    }
}

/// Parse a Happ routing profile JSON (`Name`, `GlobalProxy`, `DirectSites`, ...).
pub fn from_happ_profile(json: &Value) -> Result<ProviderRouting> {
    let obj = json
        .as_object()
        .ok_or_else(|| Error::Parse("happ profile is not a JSON object".to_string()))?;

    let mut routing = ProviderRouting {
        name: get_str(obj, &["name"]).unwrap_or_default(),
        enabled: true,
        global_proxy: get_bool(obj, &["globalproxy"]).unwrap_or(false),
        rules: Vec::new(),
        domain_strategy: get_str(obj, &["domainstrategy"]),
        dns: None,
        dns_hosts: get_string_map(obj, &["dnshosts"]),
        fake_dns: get_bool(obj, &["fakedns"]).unwrap_or(false),
        geoip_url: get_str(obj, &["geoipurl"]),
        geosite_url: get_str(obj, &["geositeurl"]),
        raw: Some(json.clone()),
    };

    // Rule order: Block, Direct, Proxy (Happ applies them in this precedence).
    push_rule(
        &mut routing.rules,
        RuleAction::Block,
        get_list(obj, &["blocksites"]),
        get_list(obj, &["blockip"]),
    );
    push_rule(
        &mut routing.rules,
        RuleAction::Direct,
        get_list(obj, &["directsites"]),
        get_list(obj, &["directip"]),
    );
    push_rule(
        &mut routing.rules,
        RuleAction::Proxy,
        get_list(obj, &["proxysites"]),
        get_list(obj, &["proxyip"]),
    );

    let remote = dns_upstream(
        get_str(obj, &["remotednsdomain"]),
        get_str(obj, &["remotednsip"]),
    );
    let direct = dns_upstream(
        get_str(obj, &["domesticdnsdomain"]),
        get_str(obj, &["domesticdnsip"]),
    );
    if remote.is_some() || direct.is_some() || routing.fake_dns {
        routing.dns = Some(Dns {
            remote: remote.unwrap_or_else(|| Dns::default().remote),
            direct: direct.unwrap_or_else(|| Dns::default().direct),
            fake_ip: routing.fake_dns,
            ..Dns::default()
        });
    }

    Ok(routing)
}

/// Parse a v2RayTun `routing` header: an Xray-style routing JSON with `rules[]`.
/// Unknown `outboundTag`s are treated as proxy; `balancers` are ignored.
pub fn from_xray_routing(json: &Value) -> Result<ProviderRouting> {
    let obj = json
        .as_object()
        .ok_or_else(|| Error::Parse("xray routing is not a JSON object".to_string()))?;

    if let Some(balancers) = obj
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("balancers"))
        .map(|(_, v)| v)
        .and_then(Value::as_array)
        .filter(|b| !b.is_empty())
    {
        tracing::warn!(
            count = balancers.len(),
            "provider routing balancers are not supported, ignored"
        );
    }

    let mut rules = Vec::new();
    if let Some(list) = find_ci(obj, "rules").and_then(Value::as_array) {
        for rule in list {
            let Some(rule) = rule.as_object() else {
                continue;
            };
            let mut domains = find_ci(rule, "domain").map(string_list).unwrap_or_default();
            domains.extend(
                find_ci(rule, "userdomain")
                    .map(string_list)
                    .unwrap_or_default(),
            );
            let ips = find_ci(rule, "ip").map(string_list).unwrap_or_default();
            if domains.is_empty() && ips.is_empty() && find_ci(rule, "port").is_none() {
                continue;
            }
            let action = find_ci(rule, "outboundtag")
                .and_then(Value::as_str)
                .map(action_from_tag)
                .unwrap_or(RuleAction::Proxy);
            let ports = find_ci(rule, "port")
                .and_then(ports_to_string)
                .filter(|p| !p.is_empty());
            let network = find_ci(rule, "network")
                .and_then(Value::as_str)
                .map(str::to_string)
                .filter(|n| !n.is_empty());
            rules.push(RoutingRule {
                domains,
                ips,
                processes: Vec::new(),
                network,
                ports,
                action,
                enabled: true,
            });
        }
    }

    Ok(ProviderRouting {
        name: get_str(obj, &["name", "id"]).unwrap_or_default(),
        enabled: true,
        global_proxy: false,
        rules,
        domain_strategy: get_str(obj, &["domainstrategy"]),
        dns: None,
        dns_hosts: BTreeMap::new(),
        fake_dns: false,
        geoip_url: None,
        geosite_url: None,
        raw: Some(json.clone()),
    })
}

fn push_rule(
    rules: &mut Vec<RoutingRule>,
    action: RuleAction,
    domains: Vec<String>,
    ips: Vec<String>,
) {
    if domains.is_empty() && ips.is_empty() {
        return;
    }
    rules.push(RoutingRule {
        domains,
        ips,
        processes: Vec::new(),
        network: None,
        ports: None,
        action,
        enabled: true,
    });
}

fn action_from_tag(tag: &str) -> RuleAction {
    match tag.to_ascii_lowercase().as_str() {
        "direct" | "direct-out" => RuleAction::Direct,
        "block" | "blocked" | "blackhole" => RuleAction::Block,
        _ => RuleAction::Proxy,
    }
}

/// `happ://routing/...` -> the part after the prefix (case-insensitive scheme).
fn strip_happ_routing(link: &str) -> Option<&str> {
    let link = link.trim();
    let prefix = "happ://routing/";
    let head = link.get(..prefix.len())?;
    head.eq_ignore_ascii_case(prefix)
        .then(|| &link[prefix.len()..])
}

/// Decode a routing payload: base64 (std/url-safe) JSON, or raw JSON with a BOM.
fn decode_profile_json(payload: &str) -> Result<Value> {
    let payload = payload.trim();
    if payload.is_empty() {
        return Err(Error::Parse("empty routing payload".to_string()));
    }
    let text =
        crate::subscription::util::decode_b64_str(payload).unwrap_or_else(|| payload.to_string());
    let text = text.trim_start_matches('\u{feff}');
    serde_json::from_str(text).map_err(|e| Error::Parse(format!("bad routing JSON: {e}")))
}

fn dns_upstream(domain: Option<String>, ip: Option<String>) -> Option<String> {
    domain
        .filter(|s| !s.is_empty())
        .or(ip.filter(|s| !s.is_empty()))
}

fn find_ci<'a>(obj: &'a serde_json::Map<String, Value>, key: &str) -> Option<&'a Value> {
    obj.iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(key))
        .map(|(_, v)| v)
}

fn get_str(obj: &serde_json::Map<String, Value>, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| find_ci(obj, key).and_then(Value::as_str))
        .map(str::to_string)
        .filter(|s| !s.is_empty())
}

fn get_bool(obj: &serde_json::Map<String, Value>, keys: &[&str]) -> Option<bool> {
    keys.iter()
        .find_map(|key| find_ci(obj, key).and_then(Value::as_bool))
}

fn get_list(obj: &serde_json::Map<String, Value>, keys: &[&str]) -> Vec<String> {
    keys.iter()
        .find_map(|key| find_ci(obj, key))
        .map(string_list)
        .unwrap_or_default()
}

fn get_string_map(obj: &serde_json::Map<String, Value>, keys: &[&str]) -> BTreeMap<String, String> {
    let Some(map) = keys
        .iter()
        .find_map(|key| find_ci(obj, key))
        .and_then(Value::as_object)
    else {
        return BTreeMap::new();
    };
    map.iter()
        .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
        .collect()
}

fn string_list(value: &Value) -> Vec<String> {
    match value {
        Value::String(s) => {
            let s = s.trim();
            if s.is_empty() {
                Vec::new()
            } else {
                vec![s.to_string()]
            }
        }
        Value::Array(items) => items
            .iter()
            .filter_map(Value::as_str)
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect(),
        _ => Vec::new(),
    }
}

fn ports_to_string(value: &Value) -> Option<String> {
    match value {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Array(items) => {
            let parts: Vec<String> = items
                .iter()
                .filter_map(|v| match v {
                    Value::Number(n) => Some(n.to_string()),
                    Value::String(s) => Some(s.clone()),
                    _ => None,
                })
                .collect();
            (!parts.is_empty()).then(|| parts.join(","))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b64(text: &str) -> String {
        use base64::engine::general_purpose::STANDARD;
        use base64::Engine;
        STANDARD.encode(text)
    }

    #[test]
    fn happ_profile_rules_block_direct_proxy_order() {
        let json: Value = serde_json::from_str(
            r#"{
                "Name": "Default",
                "GlobalProxy": true,
                "RemoteDNSType": "DoH",
                "RemoteDNSDomain": "https://1.1.1.1/dns-query",
                "RemoteDNSIP": "1.1.1.1",
                "DomesticDNSType": "DoU",
                "DomesticDNSDomain": "https://77.88.8.8/dns-query",
                "DomesticDNSIP": "77.88.8.8",
                "Geoipurl": "https://example.com/geoip.dat",
                "Geositeurl": "https://example.com/geosite.dat",
                "DnsHosts": { "example.com": "10.0.0.1" },
                "DirectSites": ["geosite:cn", "geosite:private"],
                "DirectIp": ["geoip:cn", "geoip:private"],
                "ProxySites": ["geosite:google"],
                "ProxyIp": ["geoip:google"],
                "BlockSites": ["geosite:category-ads"],
                "BlockIp": [],
                "DomainStrategy": "IPIfNonMatch",
                "FakeDNS": true,
                "LastUpdated": 1700000000
            }"#,
        )
        .unwrap();

        let routing = from_happ_profile(&json).unwrap();
        assert_eq!(routing.name, "Default");
        assert!(routing.enabled);
        assert!(routing.global_proxy);
        assert_eq!(routing.rules.len(), 3);
        assert_eq!(routing.rules[0].action, RuleAction::Block);
        assert_eq!(routing.rules[0].domains, vec!["geosite:category-ads"]);
        assert!(routing.rules[0].ips.is_empty());
        assert_eq!(routing.rules[1].action, RuleAction::Direct);
        assert_eq!(
            routing.rules[1].domains,
            vec!["geosite:cn", "geosite:private"]
        );
        assert_eq!(routing.rules[1].ips, vec!["geoip:cn", "geoip:private"]);
        assert_eq!(routing.rules[2].action, RuleAction::Proxy);
        assert_eq!(routing.rules[2].domains, vec!["geosite:google"]);
        assert_eq!(routing.domain_strategy.as_deref(), Some("IPIfNonMatch"));
        assert_eq!(
            routing.dns.as_ref().unwrap().remote,
            "https://1.1.1.1/dns-query"
        );
        assert_eq!(
            routing.dns.as_ref().unwrap().direct,
            "https://77.88.8.8/dns-query"
        );
        assert!(routing.fake_dns);
        assert_eq!(
            routing.dns_hosts.get("example.com").map(String::as_str),
            Some("10.0.0.1")
        );
        assert_eq!(
            routing.geoip_url.as_deref(),
            Some("https://example.com/geoip.dat")
        );
        assert_eq!(
            routing.geosite_url.as_deref(),
            Some("https://example.com/geosite.dat")
        );
    }

    #[test]
    fn happ_profile_tolerates_key_case_and_empty() {
        let json: Value =
            serde_json::from_str(r#"{"name":"x","blocksites":"geosite:ads","proxysites":[]}"#)
                .unwrap();
        let routing = from_happ_profile(&json).unwrap();
        assert_eq!(routing.rules.len(), 1);
        assert_eq!(routing.rules[0].action, RuleAction::Block);
        assert_eq!(routing.rules[0].domains, vec!["geosite:ads"]);
        assert!(routing.dns.is_none());
    }

    #[test]
    fn xray_routing_rules_and_unknown_outbound_is_proxy() {
        let json: Value = serde_json::from_str(
            r#"{
                "domainStrategy": "IPIfNonMatch",
                "id": "route-1",
                "name": "provider",
                "domainMatcher": "hybrid",
                "balancers": [{ "tag": "balancer" }],
                "rules": [
                    { "type": "field", "domain": ["geosite:private", "geosite:cn"], "outboundTag": "direct", "__name__": "direct" },
                    { "type": "field", "ip": ["geoip:cn"], "port": "80,443", "network": "tcp", "outboundTag": "direct" },
                    { "type": "field", "domain": ["geosite:category-ads"], "outboundTag": "block" },
                    { "type": "field", "domain": ["example.com"], "outboundTag": "custom-unknown" }
                ]
            }"#,
        )
        .unwrap();

        let routing = from_xray_routing(&json).unwrap();
        assert_eq!(routing.name, "provider");
        assert_eq!(routing.domain_strategy.as_deref(), Some("IPIfNonMatch"));
        assert_eq!(routing.rules.len(), 4);
        assert_eq!(routing.rules[0].action, RuleAction::Direct);
        assert_eq!(
            routing.rules[0].domains,
            vec!["geosite:private", "geosite:cn"]
        );
        assert_eq!(routing.rules[1].action, RuleAction::Direct);
        assert_eq!(routing.rules[1].ips, vec!["geoip:cn"]);
        assert_eq!(routing.rules[1].ports.as_deref(), Some("80,443"));
        assert_eq!(routing.rules[1].network.as_deref(), Some("tcp"));
        assert_eq!(routing.rules[2].action, RuleAction::Block);
        assert_eq!(routing.rules[3].action, RuleAction::Proxy);
    }

    #[test]
    fn xray_routing_port_array() {
        let json: Value =
            serde_json::from_str(r#"{"rules":[{"port":[53,443],"outboundTag":"direct"}]}"#)
                .unwrap();
        let routing = from_xray_routing(&json).unwrap();
        assert_eq!(routing.rules.len(), 1);
        assert_eq!(routing.rules[0].ports.as_deref(), Some("53,443"));
    }

    #[test]
    fn happ_link_add_onadd_off() {
        let profile = r#"{"Name":"P","ProxySites":["geosite:google"],"GlobalProxy":false}"#;

        match from_happ_link(&format!("happ://routing/add/{}", b64(profile))).unwrap() {
            RoutingLinkAction::Add(r) => {
                assert_eq!(r.name, "P");
                assert_eq!(r.rules.len(), 1);
                assert_eq!(r.rules[0].action, RuleAction::Proxy);
            }
            other => panic!("expected Add, got {other:?}"),
        }

        match from_happ_link(&format!("happ://routing/onadd/{}", b64(profile))).unwrap() {
            RoutingLinkAction::AddAndActivate(r) => assert_eq!(r.name, "P"),
            other => panic!("expected AddAndActivate, got {other:?}"),
        }

        assert_eq!(
            from_happ_link("happ://routing/off").unwrap(),
            RoutingLinkAction::Off
        );
    }

    #[test]
    fn happ_link_tolerates_urlsafe_bom_and_case() {
        use base64::engine::general_purpose::URL_SAFE_NO_PAD;
        use base64::Engine;
        let profile = "\u{feff}{\"nAmE\":\"Q\",\"dIrEcTsItEs\":[\"geosite:cn\"]}";
        let link = format!("happ://routing/add/{}", URL_SAFE_NO_PAD.encode(profile));
        match from_happ_link(&link).unwrap() {
            RoutingLinkAction::Add(r) => {
                assert_eq!(r.name, "Q");
                assert_eq!(r.rules[0].action, RuleAction::Direct);
            }
            other => panic!("expected Add, got {other:?}"),
        }
    }

    #[test]
    fn rejects_non_routing_links() {
        assert!(from_happ_link("happ://add/https://example.com").is_err());
        assert!(from_happ_link("happ://routing/add/%%%").is_err());
        assert!(from_happ_link("happ://routing/oops/abc").is_err());
    }
}

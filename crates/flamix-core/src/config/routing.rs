//! Core-neutral routing rules shared by the Xray and sing-box generators.
//!
//! Rules carry values in Xray syntax (`geosite:category-ru`, `geoip:ru`,
//! `domain:example.com`, `full:x.y`, `regexp:...`, `geoip:private`, CIDRs,
//! `process:...`); each core translates them into its own JSON shape.
//!
//! The "rest -> proxy" fallback of the Global/Custom presets is not emitted as a rule: it is
//! the first Xray outbound / the sing-box `route.final`, so unmatched traffic reaches the
//! proxy. The RF preset (`BypassRu`) falls back to DIRECT instead ([`final_direct`]).

use crate::config::ru_routing::{self, RuAction, RuleKind};
use crate::model::{MirrorMode, ProviderRouting, Routing, RoutingPreset, RuleAction};

/// One core-neutral routing rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Rule {
    pub domains: Vec<String>,
    pub ips: Vec<String>,
    pub processes: Vec<String>,
    /// "tcp", "udp" or empty for both.
    pub network: Option<String>,
    pub ports: Option<String>,
    pub action: RuleAction,
}

impl Rule {
    fn new(action: RuleAction) -> Self {
        Self {
            domains: Vec::new(),
            ips: Vec::new(),
            processes: Vec::new(),
            network: None,
            ports: None,
            action,
        }
    }
}

/// Enabled custom rules, in the order the user defined them.
pub(crate) fn custom_rules(routing: &Routing) -> Vec<Rule> {
    routing
        .custom_rules
        .iter()
        .filter(|r| r.enabled)
        .map(|r| Rule {
            domains: r.domains.clone(),
            ips: r.ips.clone(),
            processes: r.processes.clone(),
            network: r.network.clone(),
            ports: r.ports.clone(),
            action: r.action,
        })
        .collect()
}

/// Enabled rules of a provider routing profile, in profile order.
///
/// These go *before* the user's custom rules and replace the built-in preset.
pub(crate) fn provider_rules(provider: &ProviderRouting) -> Vec<Rule> {
    provider
        .rules
        .iter()
        .filter(|r| r.enabled)
        .map(|r| Rule {
            domains: r.domains.clone(),
            ips: r.ips.clone(),
            processes: r.processes.clone(),
            network: r.network.clone(),
            ports: r.ports.clone(),
            action: r.action,
        })
        .collect()
}

/// True when the unmatched traffic of this routing goes DIRECT (the RF preset fallback)
/// instead of through the proxy.
pub(crate) fn final_direct(routing: &Routing) -> bool {
    routing.preset == RoutingPreset::BypassRu
}

/// Rules of a built-in preset.
pub(crate) fn preset_rules(routing: &Routing) -> Vec<Rule> {
    match routing.preset {
        // Private ranges direct, everything else proxied (the fallback).
        RoutingPreset::Global => vec![Rule {
            ips: vec!["geoip:private".into()],
            ..Rule::new(RuleAction::Direct)
        }],
        RoutingPreset::BypassRu => ru_rules(routing.anixart_mirror),
        // Only the user's rules; unmatched traffic goes through the proxy.
        RoutingPreset::Custom => Vec::new(),
    }
}

/// Which `Rule` field a data rule fills. Rules are only grouped while this and the action
/// stay the same, and the kinds are never mixed in one rule (Xray ANDs the fields of a rule,
/// and so does sing-box), so the order of the data file is preserved exactly.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Slot {
    Domain(&'static str),
    Ip(&'static str),
    Port,
    Process,
}

/// The RF preset (`data/ru-routing.yaml`) as core-neutral rules, in file order.
///
/// Skipped: `rule-set` entries (remote text lists exist only in mihomo) and the final rule
/// (see [`final_direct`]).
fn ru_rules(mirror: MirrorMode) -> Vec<Rule> {
    let mut out: Vec<Rule> = Vec::new();
    let mut last: Option<(Slot, RuleAction)> = None;
    for rule in &ru_routing::data().rules {
        let action = match rule.action {
            RuAction::Direct => RuleAction::Direct,
            RuAction::Proxy => RuleAction::Proxy,
            RuAction::Mirror => match mirror {
                MirrorMode::Direct => RuleAction::Direct,
                MirrorMode::Proxy => RuleAction::Proxy,
            },
        };
        let (slot, entries): (Slot, Vec<String>) = match rule.kind {
            RuleKind::DomainSuffix => (
                Slot::Domain("suffix"),
                vec![format!("domain:{}", rule.value)],
            ),
            RuleKind::DomainKeyword => (
                Slot::Domain("keyword"),
                vec![format!("keyword:{}", rule.value)],
            ),
            RuleKind::Geosite => (
                Slot::Domain("geosite"),
                vec![format!("geosite:{}", rule.value)],
            ),
            RuleKind::Geoip => (
                Slot::Ip("geoip"),
                vec![format!("geoip:{}", rule.value.to_lowercase())],
            ),
            RuleKind::IpCidr => (Slot::Ip("cidr"), vec![rule.value.clone()]),
            RuleKind::DstPort => (Slot::Port, vec![rule.value.clone()]),
            RuleKind::ProcessRegex => (Slot::Process, rule.process_names()),
            RuleKind::RuleSet => {
                tracing::warn!(
                    "RF routing: remote rule list '{}' is mihomo-only; skipped for this core",
                    rule.value
                );
                continue;
            }
            RuleKind::Final => continue,
        };
        if last != Some((slot, action)) || out.is_empty() {
            out.push(Rule::new(action));
            last = Some((slot, action));
        }
        let Some(current) = out.last_mut() else {
            continue;
        };
        match slot {
            Slot::Domain(_) => current.domains.extend(entries),
            Slot::Ip(_) => current.ips.extend(entries),
            Slot::Port => {
                let ports = current.ports.get_or_insert_with(String::new);
                for entry in entries {
                    if !ports.is_empty() {
                        ports.push(',');
                    }
                    ports.push_str(&entry);
                }
            }
            Slot::Process => current.processes.extend(entries),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Routing, RoutingRule};

    #[test]
    fn custom_rules_skip_disabled() {
        let routing = Routing {
            custom_rules: vec![
                RoutingRule {
                    domains: vec!["domain:example.com".into()],
                    ips: vec![],
                    processes: vec![],
                    network: None,
                    ports: None,
                    action: RuleAction::Direct,
                    enabled: true,
                },
                RoutingRule {
                    domains: vec!["domain:skip.com".into()],
                    ips: vec![],
                    processes: vec![],
                    network: None,
                    ports: None,
                    action: RuleAction::Block,
                    enabled: false,
                },
            ],
            ..Routing::default()
        };
        let rules = custom_rules(&routing);
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].domains, vec!["domain:example.com".to_string()]);
    }

    #[test]
    fn rf_preset_keeps_order_and_groups() {
        let rules = preset_rules(&Routing::default());
        // First data rules: dst-port 53 direct, geoip private direct, then the direct domains.
        assert_eq!(rules[0].ports.as_deref(), Some("53"));
        assert_eq!(rules[0].action, RuleAction::Direct);
        assert_eq!(rules[1].ips, vec!["geoip:private".to_string()]);
        assert!(rules[2]
            .domains
            .contains(&"domain:boykisser.online".to_string()));
        assert_eq!(rules[2].action, RuleAction::Direct);
        // Kinds are never mixed inside one rule.
        for r in &rules {
            let kinds = [
                !r.domains.is_empty(),
                !r.ips.is_empty(),
                r.ports.is_some(),
                !r.processes.is_empty(),
            ];
            assert_eq!(kinds.iter().filter(|k| **k).count(), 1);
        }
        // Remote rule lists are skipped.
        assert!(!rules
            .iter()
            .any(|r| r.ips.iter().any(|i| i.contains("itdog"))));
        assert!(final_direct(&Routing::default()));
        assert!(preset_rules(&Routing {
            preset: RoutingPreset::Custom,
            ..Routing::default()
        })
        .is_empty());
        assert!(!final_direct(&Routing {
            preset: RoutingPreset::Global,
            ..Routing::default()
        }));
    }

    #[test]
    fn rf_mirror_follows_the_switch() {
        let find = |mirror: MirrorMode| {
            let routing = Routing {
                anixart_mirror: mirror,
                ..Routing::default()
            };
            preset_rules(&routing)
                .into_iter()
                .find(|r| r.domains.contains(&"domain:anixart.tv".to_string()))
                .unwrap()
                .action
        };
        assert_eq!(find(MirrorMode::Direct), RuleAction::Direct);
        assert_eq!(find(MirrorMode::Proxy), RuleAction::Proxy);
    }
}

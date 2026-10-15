//! Core-neutral routing rules shared by the Xray and sing-box generators.
//!
//! Rules carry values in Xray syntax (`geosite:category-ru`, `geoip:ru`,
//! `domain:example.com`, `full:x.y`, `regexp:...`, `geoip:private`, CIDRs,
//! `process:...`); each core translates them into its own JSON shape.
//!
//! The "rest -> proxy" fallback of every preset is not emitted as a rule: it is
//! the first Xray outbound / the sing-box `route.final`, so unmatched traffic
//! always reaches the proxy.

use crate::model::{Routing, RoutingPreset, RuleAction};

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

/// Rules of a built-in preset.
pub(crate) fn preset_rules(preset: RoutingPreset) -> Vec<Rule> {
    match preset {
        // Private ranges direct, everything else proxied (the fallback).
        RoutingPreset::Global => vec![Rule {
            ips: vec!["geoip:private".into()],
            ..Rule::new(RuleAction::Direct)
        }],
        RoutingPreset::BypassRu => vec![
            Rule {
                domains: vec!["geosite:category-ads-all".into()],
                ..Rule::new(RuleAction::Block)
            },
            Rule {
                domains: vec![
                    "geosite:private".into(),
                    "geosite:category-ru".into(),
                    "domain:ru".into(),
                    "domain:su".into(),
                    "domain:xn--p1ai".into(),
                ],
                ..Rule::new(RuleAction::Direct)
            },
            Rule {
                ips: vec!["geoip:private".into(), "geoip:ru".into()],
                ..Rule::new(RuleAction::Direct)
            },
        ],
        // Only the user's rules; unmatched traffic goes through the proxy.
        RoutingPreset::Custom => Vec::new(),
    }
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
    fn bypass_ru_preset_order() {
        let rules = preset_rules(RoutingPreset::BypassRu);
        assert_eq!(rules.len(), 3);
        assert_eq!(rules[0].action, RuleAction::Block);
        assert_eq!(rules[1].domains[0], "geosite:private");
        assert!(rules[2].ips.contains(&"geoip:ru".to_string()));
        assert!(preset_rules(RoutingPreset::Custom).is_empty());
    }
}

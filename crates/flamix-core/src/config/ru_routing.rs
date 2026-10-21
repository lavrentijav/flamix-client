//! The "RF" routing preset as data: `data/ru-routing.yaml`, parsed once.
//!
//! The file is a core-neutral rule list (converted from a mihomo config). Rules are ordered and
//! the first match wins; the last rule is `final` (the fallback, DIRECT for this preset).
//! Each core renders it its own way: Xray/sing-box through [`crate::config::routing`], mihomo
//! directly from these typed rules ([`crate::config::mihomo`]).

use serde::Deserialize;
use std::collections::BTreeMap;
use std::sync::OnceLock;

const RAW: &str = include_str!("../../data/ru-routing.yaml");

/// Kind of one rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum RuleKind {
    DomainSuffix,
    DomainKeyword,
    Geosite,
    Geoip,
    IpCidr,
    DstPort,
    ProcessRegex,
    RuleSet,
    Final,
}

/// Action of one rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum RuAction {
    Direct,
    Proxy,
    /// The Anixart switch: resolved through `Routing.anixart_mirror`.
    Mirror,
}

/// One rule of the data file.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub(crate) struct RuRule {
    #[serde(rename = "type")]
    pub kind: RuleKind,
    /// Absent for `final`.
    #[serde(default)]
    pub value: String,
    pub action: RuAction,
    /// mihomo select group (`DISCORD`, `ANIXART-MIRROR`).
    #[serde(default)]
    pub group: Option<String>,
}

impl RuRule {
    /// Plain process names behind a `process-regex` rule.
    ///
    /// The `names` key of the data file is not reliable (it holds regex flags), so the names
    /// are derived from the first alternation group of the regex, e.g.
    /// `(?i)^(discord|vesktop)( helper.*)?(\.exe)?$` -> `discord`, `vesktop`. Every name is
    /// returned lower-case, plus its `.exe` form (Windows process names).
    pub fn process_names(&self) -> Vec<String> {
        let body = self
            .value
            .trim_start_matches("(?i)")
            .trim_start_matches('^');
        let Some(rest) = body.strip_prefix('(') else {
            return Vec::new();
        };
        // The first group ends at its matching parenthesis.
        let mut depth = 1;
        let mut end = rest.len();
        for (i, c) in rest.char_indices() {
            match c {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        end = i;
                        break;
                    }
                }
                _ => {}
            }
        }
        let inner = &rest[..end];
        // Split on top-level `|`.
        let mut alts = Vec::new();
        let mut depth = 0;
        let mut cur = String::new();
        for c in inner.chars() {
            match c {
                '(' => {
                    depth += 1;
                    cur.push(c);
                }
                ')' => {
                    depth -= 1;
                    cur.push(c);
                }
                '|' if depth == 0 => alts.push(std::mem::take(&mut cur)),
                _ => cur.push(c),
            }
        }
        alts.push(cur);
        let mut out: Vec<String> = Vec::new();
        for alt in alts {
            // Drop optional groups such as `(\.root)?`, then unescape dots.
            let mut name = String::new();
            let mut depth = 0;
            for c in alt.chars() {
                match c {
                    '(' => depth += 1,
                    ')' => depth -= 1,
                    _ if depth == 0 => name.push(c),
                    _ => {}
                }
            }
            let name = name
                .trim_end_matches('?')
                .replace("\\.", ".")
                .to_lowercase();
            if name.is_empty() || name.contains(['\\', '*', '[', '+', '?', '(', ')']) {
                continue;
            }
            for candidate in [name.clone(), format!("{name}.exe")] {
                if !out.contains(&candidate) {
                    out.push(candidate);
                }
            }
        }
        out
    }
}

/// One remote text rule list (mihomo `rule-providers`).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub(crate) struct RuleProvider {
    pub url: String,
    pub behavior: String,
    pub format: String,
    pub interval: u32,
}

/// The whole data file.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct RuRouting {
    #[allow(dead_code)]
    pub version: u32,
    #[allow(dead_code)]
    pub source: String,
    pub rule_providers: BTreeMap<String, RuleProvider>,
    pub rules: Vec<RuRule>,
}

/// The parsed RF routing data. Panics only if the file embedded at build time is malformed,
/// which the unit tests rule out.
pub(crate) fn data() -> &'static RuRouting {
    static DATA: OnceLock<RuRouting> = OnceLock::new();
    DATA.get_or_init(|| match serde_yaml::from_str(RAW) {
        Ok(data) => data,
        Err(e) => {
            tracing::error!("embedded ru-routing.yaml is invalid: {e}");
            RuRouting {
                version: 0,
                source: String::new(),
                rule_providers: BTreeMap::new(),
                rules: Vec::new(),
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn count(kind: RuleKind) -> usize {
        data().rules.iter().filter(|r| r.kind == kind).count()
    }

    #[test]
    fn loads_with_expected_counts() {
        let d = data();
        assert_eq!(d.rules.len(), 391);
        assert_eq!(count(RuleKind::DomainSuffix), 347);
        assert_eq!(count(RuleKind::DomainKeyword), 1);
        assert_eq!(count(RuleKind::Geosite), 22);
        assert_eq!(count(RuleKind::Geoip), 3);
        assert_eq!(count(RuleKind::IpCidr), 8);
        assert_eq!(count(RuleKind::DstPort), 1);
        assert_eq!(count(RuleKind::ProcessRegex), 5);
        assert_eq!(count(RuleKind::RuleSet), 3);
        assert_eq!(count(RuleKind::Final), 1);
        assert_eq!(d.rule_providers.len(), 3);
    }

    #[test]
    fn final_is_last_and_direct() {
        let last = data().rules.last().unwrap();
        assert_eq!(last.kind, RuleKind::Final);
        assert_eq!(last.action, RuAction::Direct);
    }

    #[test]
    fn rule_sets_reference_providers() {
        for r in data().rules.iter().filter(|r| r.kind == RuleKind::RuleSet) {
            assert!(data().rule_providers.contains_key(&r.value), "{}", r.value);
        }
    }

    #[test]
    fn process_names_from_regex() {
        let discord = data()
            .rules
            .iter()
            .find(|r| r.kind == RuleKind::ProcessRegex && r.group.as_deref() == Some("DISCORD"))
            .unwrap();
        let names = discord.process_names();
        assert!(names.contains(&"discord".to_string()));
        assert!(names.contains(&"vesktop.exe".to_string()));
        let tg = data()
            .rules
            .iter()
            .find(|r| r.value.contains("telegram"))
            .unwrap();
        let names = tg.process_names();
        assert!(names.contains(&"telegram desktop".to_string()));
        assert!(names.contains(&"whatsapp".to_string()));
        assert!(names.contains(&"zoom.us".to_string()));
        assert!(!names.iter().any(|n| n.contains('\\') || n.contains('(')));
    }
}

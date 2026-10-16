//! "Маршрутизация" page (page 2). OWNER: T17C (docs/tasks/T17-pages.md).
//!
//! Rust talks to the page only through the [`RoutingBridge`] global. Every change is written
//! through [`Controller::mutate`] and a reconnect (if a core is running) is scheduled with a
//! 700 ms debounce. All non-trivial logic is pure and unit-tested below.

use crate::controller::Controller;
use crate::{AppWindow, ProviderRoutingItem, RoutingBridge, RoutingRuleItem};
use flamix_core::model::{AppState, Routing, RoutingPreset, RoutingRule, RuleAction, Subscription};
use slint::ComponentHandle;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

/// Delay before applying a routing change to a running core.
const RECONNECT_DEBOUNCE: Duration = Duration::from_millis(700);

/// Domain strategy labels, used for the ComboBox mapping.
const STRATEGIES: [&str; 3] = ["AsIs", "IPIfNonMatch", "IPOnDemand"];

pub fn init(ui: &AppWindow, ctrl: Option<&Arc<Controller>>) {
    match ctrl {
        Some(ctrl) => wire(ui, Arc::clone(ctrl)),
        None => fill_demo(ui),
    }
}

// --- pure helpers ---------------------------------------------------------

fn preset_index(preset: RoutingPreset) -> i32 {
    match preset {
        RoutingPreset::Global => 0,
        RoutingPreset::BypassRu => 1,
        RoutingPreset::Custom => 2,
    }
}

fn preset_from_index(index: i32) -> RoutingPreset {
    match index {
        0 => RoutingPreset::Global,
        2 => RoutingPreset::Custom,
        _ => RoutingPreset::BypassRu,
    }
}

fn strategy_index(strategy: &str) -> i32 {
    STRATEGIES
        .iter()
        .position(|s| *s == strategy)
        .map(|i| i as i32)
        .unwrap_or(1)
}

fn strategy_from_label(label: &str) -> Option<&'static str> {
    STRATEGIES.iter().copied().find(|s| *s == label)
}

fn action_index(action: RuleAction) -> i32 {
    match action {
        RuleAction::Proxy => 0,
        RuleAction::Direct => 1,
        RuleAction::Block => 2,
    }
}

fn action_from_index(index: i32) -> RuleAction {
    match index {
        0 => RuleAction::Proxy,
        1 => RuleAction::Direct,
        _ => RuleAction::Block,
    }
}

/// Which value list a rule currently edits. Domains win over IPs win over processes.
fn kind_index(rule: &RoutingRule) -> usize {
    if !rule.ips.is_empty() {
        1
    } else if !rule.processes.is_empty() {
        2
    } else {
        0
    }
}

fn kind_from_index(index: i32) -> Option<usize> {
    match index {
        0 => Some(0),
        1 => Some(1),
        2 => Some(2),
        _ => None,
    }
}

/// Split a UI value into trimmed, non-empty entries.
///
/// Entries are separated by commas **or** newlines (`\n`/`\r`, so CRLF works too): the user may
/// type a comma-separated line and it is re-rendered one entry per line on the next refresh.
/// Order is preserved and duplicates are kept.
fn split_values(text: &str) -> Vec<String> {
    text.split([',', '\n', '\r'])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

/// Render a rule's value list as one entry per line (the multi-line input format).
fn join_values(rule: &RoutingRule, kind: usize) -> String {
    let list = match kind {
        1 => &rule.ips,
        2 => &rule.processes,
        _ => &rule.domains,
    };
    list.join("\n")
}

/// Replace the rule's values with `text`, keeping exactly one kind populated.
fn set_rule_values(rule: &mut RoutingRule, kind: usize, text: &str) {
    let values = split_values(text);
    rule.domains.clear();
    rule.ips.clear();
    rule.processes.clear();
    match kind {
        1 => rule.ips = values,
        2 => rule.processes = values,
        _ => rule.domains = values,
    }
}

/// Move the current values to another kind, so switching kind never hides data.
fn change_rule_kind(rule: &mut RoutingRule, kind: usize) {
    let current = kind_index(rule);
    if current == kind {
        return;
    }
    let text = join_values(rule, current);
    set_rule_values(rule, kind, &text);
}

fn is_empty_rule(rule: &RoutingRule) -> bool {
    rule.domains.is_empty() && rule.ips.is_empty() && rule.processes.is_empty()
}

/// First validation problem of the custom rules ("" when valid).
fn validation_error(routing: &Routing) -> String {
    for (index, rule) in routing.custom_rules.iter().enumerate() {
        if rule.enabled && is_empty_rule(rule) {
            return format!("Правило {}: укажите значения", index + 1);
        }
    }
    String::new()
}

/// Russian plural for a rule count ("1 правило", "2 правила", "5 правил").
fn rules_label(count: usize) -> String {
    let word = if (11..=14).contains(&(count % 100)) {
        "правил"
    } else {
        match count % 10 {
            1 => "правило",
            2..=4 => "правила",
            _ => "правил",
        }
    };
    format!("{count} {word}")
}

fn rule_mut(routing: &mut Routing, index: i32) -> Option<&mut RoutingRule> {
    usize::try_from(index)
        .ok()
        .and_then(|i| routing.custom_rules.get_mut(i))
}

/// Swap a rule with its neighbour `delta` places away. Returns whether anything moved.
/// Both ends of the swap are bounds-checked (`from` included: an out-of-range index must not
/// panic on the other side even when `to` happens to be valid).
fn move_rule(rules: &mut [RoutingRule], index: i32, delta: i32) -> bool {
    let Ok(from) = usize::try_from(index) else {
        return false;
    };
    if from >= rules.len() {
        return false;
    }
    let to = from as i64 + i64::from(delta);
    if to < 0 || to >= rules.len() as i64 {
        return false;
    }
    rules.swap(from, to as usize);
    true
}

/// Remembers the chip kind chosen for each rule.
///
/// The model cannot tell an empty `domains` list from an empty `ips`/`processes` list, so
/// [`kind_index`] always reports 0 for a rule with no values. Without this cache an empty rule
/// would snap back to "Домены" on every repaint (structural edits, enable toggle, ...).
#[derive(Default)]
struct KindCache {
    kinds: Vec<i32>,
}

impl KindCache {
    /// Align the cache with the current rules: keep the chosen kind for empty rules, take the
    /// authoritative derived kind for rules that already contain values.
    fn sync(&mut self, routing: &Routing) {
        let count = routing.custom_rules.len();
        self.kinds.resize(count, 0);
        for (index, rule) in routing.custom_rules.iter().enumerate() {
            if !is_empty_rule(rule) {
                self.kinds[index] = kind_index(rule) as i32;
            }
        }
    }

    fn set(&mut self, index: i32, kind: i32) {
        if let Some(slot) = usize::try_from(index)
            .ok()
            .and_then(|index| self.kinds.get_mut(index))
        {
            *slot = kind;
        }
    }

    /// Mirror a reorder of the model (uses the same bounds rules as [`move_rule`]).
    fn swap(&mut self, index: i32, delta: i32) {
        let Ok(from) = usize::try_from(index) else {
            return;
        };
        if from >= self.kinds.len() {
            return;
        }
        let to = from as i64 + i64::from(delta);
        if to < 0 || to >= self.kinds.len() as i64 {
            return;
        }
        self.kinds.swap(from, to as usize);
    }

    fn remove(&mut self, index: i32) {
        if let Some(index) = usize::try_from(index)
            .ok()
            .filter(|index| *index < self.kinds.len())
        {
            self.kinds.remove(index);
        }
    }
}

fn rule_item(index: usize, rule: &RoutingRule, preferred_kind: i32) -> RoutingRuleItem {
    let kind = if is_empty_rule(rule) {
        kind_from_index(preferred_kind).unwrap_or(0)
    } else {
        kind_index(rule)
    };
    RoutingRuleItem {
        index: index as i32,
        action: action_index(rule.action),
        kind: kind as i32,
        values: join_values(rule, kind).into(),
        enabled: rule.enabled,
    }
}

fn build_rule_items(routing: &Routing, kinds: &[i32]) -> Vec<RoutingRuleItem> {
    routing
        .custom_rules
        .iter()
        .enumerate()
        .map(|(index, rule)| rule_item(index, rule, kinds.get(index).copied().unwrap_or(0)))
        .collect()
}

/// Number of rules that actually take effect: none while the profile is disabled or the user
/// ignores it, otherwise only the individual enabled rules.
fn effective_rule_count(sub: &Subscription) -> usize {
    let Some(routing) = sub.provider.routing.as_ref() else {
        return 0;
    };
    if !routing.enabled || sub.options.ignore_provider_routing {
        return 0;
    }
    routing.rules.iter().filter(|rule| rule.enabled).count()
}

fn provider_items(state: &AppState) -> Vec<ProviderRoutingItem> {
    state
        .subscriptions
        .iter()
        .filter_map(|sub| {
            let routing = sub.provider.routing.as_ref()?;
            let name = if routing.name.trim().is_empty() {
                sub.name.clone()
            } else {
                routing.name.clone()
            };
            Some(ProviderRoutingItem {
                id: sub.id.clone().into(),
                name: name.into(),
                rules_text: rules_label(effective_rule_count(sub)).into(),
                enabled: routing.enabled,
                ignored: sub.options.ignore_provider_routing,
            })
        })
        .collect()
}

// --- bridge publishing ----------------------------------------------------

fn publish(
    ui: &AppWindow,
    routing: &Routing,
    rules: Vec<RoutingRuleItem>,
    providers: Vec<ProviderRoutingItem>,
    error: String,
) {
    let bridge = ui.global::<RoutingBridge>();
    bridge.set_preset(preset_index(routing.preset));
    bridge.set_domain_strategy(strategy_index(&routing.domain_strategy));
    bridge.set_block_quic(routing.block_quic);
    bridge.set_rules(crate::view::model(rules));
    bridge.set_has_providers(!providers.is_empty());
    bridge.set_providers(crate::view::model(providers));
    bridge.set_error_text(error.into());
}

fn refresh(ui: &AppWindow, ctrl: &Controller, cache: &Rc<RefCell<KindCache>>) {
    let (routing, providers) = ctrl.with_state(|s| (s.settings.routing.clone(), provider_items(s)));
    let error = validation_error(&routing);
    let mut cache = cache.borrow_mut();
    cache.sync(&routing);
    let rules = build_rule_items(&routing, &cache.kinds);
    publish(ui, &routing, rules, providers, error);
}

fn repaint(weak: &slint::Weak<AppWindow>, ctrl: &Controller, cache: &Rc<RefCell<KindCache>>) {
    if let Some(ui) = weak.upgrade() {
        refresh(&ui, ctrl, cache);
    }
}

/// Debounced `reconnect_if_connected`: every change bumps a generation, only the last wins.
#[derive(Clone)]
struct Defer {
    ctrl: Arc<Controller>,
    generation: Rc<Cell<u64>>,
}

impl Defer {
    fn new(ctrl: Arc<Controller>) -> Self {
        Self {
            ctrl,
            generation: Rc::new(Cell::new(0)),
        }
    }

    fn schedule(&self) {
        let generation = self.generation.get().wrapping_add(1);
        self.generation.set(generation);
        let ctrl = Arc::clone(&self.ctrl);
        let shared = Rc::clone(&self.generation);
        slint::Timer::single_shot(RECONNECT_DEBOUNCE, move || {
            if shared.get() == generation {
                ctrl.reconnect_if_connected();
            }
        });
    }
}

// --- live wiring ----------------------------------------------------------

fn wire(ui: &AppWindow, ctrl: Arc<Controller>) {
    let cache: Rc<RefCell<KindCache>> = Rc::new(RefCell::new(KindCache::default()));
    refresh(ui, &ctrl, &cache);
    let defer = Defer::new(Arc::clone(&ctrl));

    {
        let weak = ui.as_weak();
        let ctrl = Arc::clone(&ctrl);
        let cache = Rc::clone(&cache);
        let defer = defer.clone();
        ui.global::<RoutingBridge>()
            .on_preset_selected(move |index| {
                ctrl.mutate(|s| s.settings.routing.preset = preset_from_index(index));
                repaint(&weak, &ctrl, &cache);
                defer.schedule();
            });
    }
    {
        let weak = ui.as_weak();
        let ctrl = Arc::clone(&ctrl);
        let cache = Rc::clone(&cache);
        let defer = defer.clone();
        ui.global::<RoutingBridge>()
            .on_rule_action_changed(move |index, action| {
                ctrl.mutate(|s| {
                    if let Some(rule) = rule_mut(&mut s.settings.routing, index) {
                        rule.action = action_from_index(action);
                    }
                });
                repaint(&weak, &ctrl, &cache);
                defer.schedule();
            });
    }
    {
        let ctrl = Arc::clone(&ctrl);
        let cache = Rc::clone(&cache);
        let defer = defer.clone();
        ui.global::<RoutingBridge>()
            .on_rule_kind_changed(move |index, kind| {
                let Some(kind) = kind_from_index(kind) else {
                    return;
                };
                ctrl.mutate(|s| {
                    if let Some(rule) = rule_mut(&mut s.settings.routing, index) {
                        change_rule_kind(rule, kind);
                    }
                });
                cache.borrow_mut().set(index, kind as i32);
                // No refresh: keep the chips on the chosen kind so the next edit lands in it.
                defer.schedule();
            });
    }
    {
        let ctrl = Arc::clone(&ctrl);
        let cache = Rc::clone(&cache);
        let defer = defer.clone();
        ui.global::<RoutingBridge>()
            .on_rule_values_changed(move |index, kind, text| {
                let kind = kind_from_index(kind).unwrap_or(0);
                ctrl.mutate(|s| {
                    if let Some(rule) = rule_mut(&mut s.settings.routing, index) {
                        set_rule_values(rule, kind, text.as_str());
                    }
                });
                cache.borrow_mut().set(index, kind as i32);
                // No refresh: preserve the LineEdit cursor while typing.
                defer.schedule();
            });
    }
    {
        let weak = ui.as_weak();
        let ctrl = Arc::clone(&ctrl);
        let cache = Rc::clone(&cache);
        let defer = defer.clone();
        ui.global::<RoutingBridge>()
            .on_rule_enabled_changed(move |index, enabled| {
                ctrl.mutate(|s| {
                    if let Some(rule) = rule_mut(&mut s.settings.routing, index) {
                        rule.enabled = enabled;
                    }
                });
                repaint(&weak, &ctrl, &cache);
                defer.schedule();
            });
    }
    {
        let weak = ui.as_weak();
        let ctrl = Arc::clone(&ctrl);
        let cache = Rc::clone(&cache);
        let defer = defer.clone();
        ui.global::<RoutingBridge>()
            .on_rule_move(move |index, delta| {
                ctrl.mutate(|s| {
                    move_rule(&mut s.settings.routing.custom_rules, index, delta);
                });
                cache.borrow_mut().swap(index, delta);
                repaint(&weak, &ctrl, &cache);
                defer.schedule();
            });
    }
    {
        let weak = ui.as_weak();
        let ctrl = Arc::clone(&ctrl);
        let cache = Rc::clone(&cache);
        let defer = defer.clone();
        ui.global::<RoutingBridge>().on_rule_delete(move |index| {
            ctrl.mutate(|s| {
                if let Ok(index) = usize::try_from(index) {
                    if index < s.settings.routing.custom_rules.len() {
                        s.settings.routing.custom_rules.remove(index);
                    }
                }
            });
            cache.borrow_mut().remove(index);
            repaint(&weak, &ctrl, &cache);
            defer.schedule();
        });
    }
    {
        let weak = ui.as_weak();
        let ctrl = Arc::clone(&ctrl);
        let cache = Rc::clone(&cache);
        let defer = defer.clone();
        ui.global::<RoutingBridge>().on_rule_add(move || {
            ctrl.mutate(|s| {
                s.settings.routing.custom_rules.push(RoutingRule {
                    domains: Vec::new(),
                    ips: Vec::new(),
                    processes: Vec::new(),
                    network: None,
                    ports: None,
                    action: RuleAction::Proxy,
                    enabled: true,
                });
            });
            repaint(&weak, &ctrl, &cache);
            defer.schedule();
        });
    }
    {
        let weak = ui.as_weak();
        let ctrl = Arc::clone(&ctrl);
        let cache = Rc::clone(&cache);
        let defer = defer.clone();
        ui.global::<RoutingBridge>()
            .on_domain_strategy_changed(move |label| {
                let Some(strategy) = strategy_from_label(label.as_str()) else {
                    return;
                };
                ctrl.mutate(|s| s.settings.routing.domain_strategy = strategy.to_string());
                repaint(&weak, &ctrl, &cache);
                defer.schedule();
            });
    }
    {
        let weak = ui.as_weak();
        let ctrl = Arc::clone(&ctrl);
        let cache = Rc::clone(&cache);
        let defer = defer.clone();
        ui.global::<RoutingBridge>()
            .on_block_quic_changed(move |enabled| {
                ctrl.mutate(|s| s.settings.routing.block_quic = enabled);
                repaint(&weak, &ctrl, &cache);
                defer.schedule();
            });
    }
    {
        let weak = ui.as_weak();
        let ctrl = Arc::clone(&ctrl);
        let cache = Rc::clone(&cache);
        let defer = defer.clone();
        ui.global::<RoutingBridge>()
            .on_provider_enabled_changed(move |id, enabled| {
                ctrl.mutate(|s| {
                    if let Some(sub) = subscription_mut(s, id.as_str()) {
                        if let Some(routing) = sub.provider.routing.as_mut() {
                            routing.enabled = enabled;
                        }
                    }
                });
                repaint(&weak, &ctrl, &cache);
                defer.schedule();
            });
    }
    {
        let weak = ui.as_weak();
        let ctrl = Arc::clone(&ctrl);
        let cache = Rc::clone(&cache);
        let defer = defer.clone();
        ui.global::<RoutingBridge>()
            .on_provider_ignored_changed(move |id, ignored| {
                ctrl.mutate(|s| {
                    if let Some(sub) = subscription_mut(s, id.as_str()) {
                        sub.options.ignore_provider_routing = ignored;
                    }
                });
                repaint(&weak, &ctrl, &cache);
                defer.schedule();
            });
    }
}

fn subscription_mut<'a>(state: &'a mut AppState, id: &str) -> Option<&'a mut Subscription> {
    state.subscriptions.iter_mut().find(|s| s.id == id)
}

// --- demo / screenshot data ----------------------------------------------

fn demo_rule(
    action: RuleAction,
    domains: &[&str],
    ips: &[&str],
    processes: &[&str],
) -> RoutingRule {
    RoutingRule {
        domains: domains.iter().map(|s| (*s).to_string()).collect(),
        ips: ips.iter().map(|s| (*s).to_string()).collect(),
        processes: processes.iter().map(|s| (*s).to_string()).collect(),
        network: None,
        ports: None,
        action,
        enabled: true,
    }
}

fn demo_routing() -> Routing {
    Routing {
        preset: RoutingPreset::Custom,
        custom_rules: vec![
            demo_rule(
                RuleAction::Proxy,
                &["domain:openai.com", "geosite:google"],
                &[],
                &[],
            ),
            demo_rule(
                RuleAction::Direct,
                &[],
                &["geoip:ru", "10.0.0.0/8", "192.168.0.0/16"],
                &[],
            ),
            demo_rule(RuleAction::Block, &["geosite:category-ads-all"], &[], &[]),
            demo_rule(
                RuleAction::Direct,
                &[],
                &[],
                &["telegram.exe", "Discord.exe"],
            ),
        ],
        domain_strategy: "IPIfNonMatch".into(),
        block_quic: true,
    }
}

fn demo_providers() -> Vec<ProviderRoutingItem> {
    vec![
        ProviderRoutingItem {
            id: "sub-premium".into(),
            name: "Flamix Premium".into(),
            rules_text: rules_label(23).into(),
            enabled: true,
            ignored: false,
        },
        ProviderRoutingItem {
            id: "sub-basic".into(),
            name: "Базовый тариф".into(),
            rules_text: rules_label(4).into(),
            enabled: false,
            ignored: true,
        },
    ]
}

/// `--demo` / `--screenshot` mode: realistic static data, callbacks are not wired.
fn fill_demo(ui: &AppWindow) {
    let routing = demo_routing();
    let rules = build_rule_items(&routing, &[]);
    publish(ui, &routing, rules, demo_providers(), String::new());
}

#[cfg(test)]
mod tests {
    use super::*;
    use flamix_core::model::{ProviderRouting, SubscriptionOptions};

    fn subscription(id: &str, name: &str) -> Subscription {
        Subscription {
            id: id.to_string(),
            name: name.to_string(),
            auto_title: None,
            url: "https://example.com/sub".to_string(),
            backup_urls: Vec::new(),
            last_working_url: None,
            update_interval_hours: 0,
            last_update: None,
            last_error: None,
            traffic: None,
            support_url: None,
            web_page_url: None,
            announce: None,
            user_agent: None,
            pinned: false,
            collapsed: false,
            servers: Vec::new(),
            provider: Default::default(),
            options: SubscriptionOptions::default(),
        }
    }

    #[test]
    fn preset_mapping_roundtrips() {
        for preset in [
            RoutingPreset::Global,
            RoutingPreset::BypassRu,
            RoutingPreset::Custom,
        ] {
            assert_eq!(preset_from_index(preset_index(preset)), preset);
        }
        assert_eq!(preset_from_index(99), RoutingPreset::BypassRu);
    }

    #[test]
    fn strategy_mapping() {
        assert_eq!(strategy_index("AsIs"), 0);
        assert_eq!(strategy_index("IPIfNonMatch"), 1);
        assert_eq!(strategy_index("IPOnDemand"), 2);
        assert_eq!(strategy_index("weird"), 1);
        assert_eq!(strategy_from_label("IPOnDemand"), Some("IPOnDemand"));
        assert_eq!(strategy_from_label("nope"), None);
    }

    #[test]
    fn action_mapping_is_index_aligned() {
        assert_eq!(action_index(RuleAction::Proxy), 0);
        assert_eq!(action_index(RuleAction::Direct), 1);
        assert_eq!(action_index(RuleAction::Block), 2);
        assert_eq!(action_from_index(0), RuleAction::Proxy);
        assert_eq!(action_from_index(1), RuleAction::Direct);
        assert_eq!(action_from_index(2), RuleAction::Block);
        assert_eq!(action_from_index(-5), RuleAction::Block);
    }

    #[test]
    fn kind_detection_and_mapping() {
        let domains = demo_rule(RuleAction::Proxy, &["a.com"], &[], &[]);
        let ips = demo_rule(RuleAction::Proxy, &[], &["geoip:ru"], &[]);
        let procs = demo_rule(RuleAction::Proxy, &[], &[], &["x.exe"]);
        let empty = demo_rule(RuleAction::Proxy, &[], &[], &[]);
        assert_eq!(kind_index(&domains), 0);
        assert_eq!(kind_index(&ips), 1);
        assert_eq!(kind_index(&procs), 2);
        assert_eq!(kind_index(&empty), 0);
        assert_eq!(kind_from_index(2), Some(2));
        assert_eq!(kind_from_index(3), None);
    }

    #[test]
    fn split_and_join_values() {
        assert_eq!(
            split_values(" geosite:ru , domain:x.com,, 10.0.0.0/8 "),
            vec!["geosite:ru", "domain:x.com", "10.0.0.0/8"]
        );
        assert!(split_values("  ,  ").is_empty());
        let rule = demo_rule(RuleAction::Direct, &[], &["geoip:ru", "geoip:cn"], &[]);
        assert_eq!(join_values(&rule, 1), "geoip:ru\ngeoip:cn");
        assert_eq!(join_values(&rule, 0), "");
    }

    #[test]
    fn split_values_normalizes_commas_to_newlines() {
        // Commas, newlines and blank lines all yield the same ordered list ...
        assert_eq!(
            split_values("a.com,b.com\nc.com"),
            vec!["a.com", "b.com", "c.com"]
        );
        // ... and the rule is re-rendered one entry per line.
        let mut rule = demo_rule(RuleAction::Proxy, &[], &[], &[]);
        set_rule_values(&mut rule, 0, "a.com, b.com\n\n ,c.com\r\nd.com\r\n");
        assert_eq!(rule.domains, vec!["a.com", "b.com", "c.com", "d.com"]);
        assert_eq!(join_values(&rule, 0), "a.com\nb.com\nc.com\nd.com");
    }

    #[test]
    fn split_values_handles_crlf_and_blank_lines() {
        assert_eq!(
            split_values("geosite:ru\r\n\r\ngeoip:ru\r\n"),
            vec!["geosite:ru", "geoip:ru"]
        );
        assert!(split_values("\n\r\n,\n").is_empty());
    }

    #[test]
    fn set_rule_values_keeps_single_kind() {
        let mut rule = demo_rule(RuleAction::Proxy, &["old.com"], &[], &[]);
        set_rule_values(&mut rule, 1, "geoip:ru, 10.0.0.0/8");
        assert!(rule.domains.is_empty());
        assert!(rule.processes.is_empty());
        assert_eq!(rule.ips, vec!["geoip:ru", "10.0.0.0/8"]);
    }

    #[test]
    fn change_rule_kind_moves_existing_values() {
        let mut rule = demo_rule(RuleAction::Proxy, &["a.com", "b.com"], &[], &[]);
        change_rule_kind(&mut rule, 1);
        assert!(rule.domains.is_empty());
        assert_eq!(rule.ips, vec!["a.com", "b.com"]);
        // The moved values survive the multi-line round-trip.
        assert_eq!(join_values(&rule, 1), "a.com\nb.com");
        // Switching to the same kind is a no-op.
        change_rule_kind(&mut rule, 1);
        assert_eq!(rule.ips, vec!["a.com", "b.com"]);
        // And a differently-shaped list moves just as well.
        let mut mixed = demo_rule(RuleAction::Proxy, &[], &[], &["p.exe"]);
        change_rule_kind(&mut mixed, 0);
        assert!(mixed.processes.is_empty());
        assert_eq!(mixed.domains, vec!["p.exe"]);
    }

    #[test]
    fn validation_flags_only_enabled_empty_rules() {
        let mut routing = Routing::default();
        routing
            .custom_rules
            .push(demo_rule(RuleAction::Proxy, &[], &[], &[]));
        routing.custom_rules[0].enabled = false;
        assert_eq!(validation_error(&routing), "");
        routing.custom_rules[0].enabled = true;
        assert_eq!(validation_error(&routing), "Правило 1: укажите значения");
        routing
            .custom_rules
            .push(demo_rule(RuleAction::Direct, &["a.com"], &[], &[]));
        assert_eq!(validation_error(&routing), "Правило 1: укажите значения");
    }

    #[test]
    fn rules_label_pluralizes() {
        assert_eq!(rules_label(1), "1 правило");
        assert_eq!(rules_label(2), "2 правила");
        assert_eq!(rules_label(5), "5 правил");
        assert_eq!(rules_label(11), "11 правил");
        assert_eq!(rules_label(21), "21 правило");
        assert_eq!(rules_label(23), "23 правила");
    }

    #[test]
    fn move_rule_swaps_and_clamps() {
        let mut rules = vec![
            demo_rule(RuleAction::Proxy, &["a"], &[], &[]),
            demo_rule(RuleAction::Proxy, &["b"], &[], &[]),
            demo_rule(RuleAction::Proxy, &["c"], &[], &[]),
        ];
        assert!(move_rule(&mut rules, 0, 1));
        assert_eq!(rules[0].domains, vec!["b"]);
        assert_eq!(rules[1].domains, vec!["a"]);
        // Out of bounds is ignored on either end.
        assert!(!move_rule(&mut rules, 0, -1));
        assert!(!move_rule(&mut rules, 2, 1));
        assert_eq!(rules[0].domains, vec!["b"]);
        assert_eq!(rules[2].domains, vec!["c"]);
    }

    #[test]
    fn move_rule_from_out_of_range_does_not_panic() {
        let mut rules = vec![
            demo_rule(RuleAction::Proxy, &["a"], &[], &[]),
            demo_rule(RuleAction::Proxy, &["b"], &[], &[]),
        ];
        // `from == len` while `to` is a valid index: `from` must be checked before swapping.
        assert!(!move_rule(&mut rules, 2, -1));
        assert!(!move_rule(&mut rules, 5, -2));
        assert!(!move_rule(&mut rules, -1, 1));
        assert_eq!(rules[0].domains, vec!["a"]);
        assert_eq!(rules[1].domains, vec!["b"]);
    }

    #[test]
    fn rule_item_reflects_model() {
        let rule = demo_rule(RuleAction::Block, &["geosite:ads"], &[], &[]);
        let item = rule_item(3, &rule, 0);
        assert_eq!(item.index, 3);
        assert_eq!(item.action, 2);
        assert_eq!(item.kind, 0);
        assert_eq!(item.values.as_str(), "geosite:ads");
        assert!(item.enabled);
    }

    #[test]
    fn rule_item_renders_values_one_per_line() {
        let rule = demo_rule(
            RuleAction::Direct,
            &[],
            &["geoip:ru", "10.0.0.0/8", "192.168.0.0/16"],
            &[],
        );
        let item = rule_item(0, &rule, 0);
        assert_eq!(item.kind, 1);
        assert_eq!(item.values.as_str(), "geoip:ru\n10.0.0.0/8\n192.168.0.0/16");
    }

    #[test]
    fn rule_item_keeps_preferred_kind_for_empty_rule() {
        let empty = demo_rule(RuleAction::Proxy, &[], &[], &[]);
        let item = rule_item(0, &empty, 2);
        assert_eq!(item.kind, 2);
        assert_eq!(item.values.as_str(), "");
        // A populated rule ignores the preferred kind and uses the real one.
        let ips = demo_rule(RuleAction::Proxy, &[], &["geoip:ru"], &[]);
        assert_eq!(rule_item(0, &ips, 2).kind, 1);
    }

    #[test]
    fn build_rule_items_maps_every_rule_and_uses_kinds() {
        let routing = Routing {
            custom_rules: vec![
                demo_rule(RuleAction::Proxy, &["a.com"], &[], &[]),
                demo_rule(RuleAction::Block, &[], &[], &[]),
            ],
            ..Routing::default()
        };
        let items = build_rule_items(&routing, &[0, 2]);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].index, 0);
        assert_eq!(items[0].values.as_str(), "a.com");
        assert_eq!(items[1].index, 1);
        assert_eq!(items[1].kind, 2);
        // A missing kind entry falls back to domains.
        let items = build_rule_items(&routing, &[]);
        assert_eq!(items[1].kind, 0);
    }

    #[test]
    fn kind_cache_sync_set_swap_remove() {
        let mut routing = Routing {
            custom_rules: vec![
                demo_rule(RuleAction::Proxy, &[], &[], &[]),
                demo_rule(RuleAction::Proxy, &[], &[], &["p.exe"]),
            ],
            ..Routing::default()
        };
        let mut cache = KindCache::default();
        cache.sync(&routing);
        // Domains for the empty rule, processes derived for the populated one.
        assert_eq!(cache.kinds, vec![0, 2]);
        cache.set(0, 1);
        cache.sync(&routing);
        assert_eq!(cache.kinds, vec![1, 2]);
        cache.swap(0, 1);
        assert_eq!(cache.kinds, vec![2, 1]);
        cache.remove(0);
        assert_eq!(cache.kinds, vec![1]);
        // Shrinking to zero and regrowing fills with the default kind.
        routing.custom_rules.clear();
        cache.sync(&routing);
        assert!(cache.kinds.is_empty());
    }

    #[test]
    fn subscription_mut_finds_and_mutates_only_the_matching_id() {
        let mut state = AppState::default();
        state.subscriptions.push(subscription("a", "Alpha"));
        state.subscriptions.push(subscription("b", "Beta"));
        assert!(subscription_mut(&mut state, "missing").is_none());
        let sub = subscription_mut(&mut state, "b").expect("b exists");
        sub.name = "Beta2".to_string();
        assert_eq!(state.subscriptions[1].name, "Beta2");
        assert_eq!(state.subscriptions[0].name, "Alpha");
    }

    #[test]
    fn effective_rule_count_counts_only_enabled_and_active() {
        let mut sub = subscription("s", "S");
        sub.provider.routing = Some(ProviderRouting {
            enabled: true,
            rules: vec![
                demo_rule(RuleAction::Proxy, &["x"], &[], &[]),
                demo_rule(RuleAction::Direct, &[], &["y"], &[]),
            ],
            ..ProviderRouting::default()
        });
        assert_eq!(effective_rule_count(&sub), 2);
        sub.provider.routing.as_mut().unwrap().rules[1].enabled = false;
        assert_eq!(effective_rule_count(&sub), 1);
        sub.provider.routing.as_mut().unwrap().enabled = false;
        assert_eq!(effective_rule_count(&sub), 0);
        sub.provider.routing.as_mut().unwrap().enabled = true;
        sub.options.ignore_provider_routing = true;
        assert_eq!(effective_rule_count(&sub), 0);
    }

    #[test]
    fn provider_items_only_include_subscriptions_with_routing() {
        let mut state = AppState::default();
        state.subscriptions.push(subscription("a", "Alpha"));
        let mut with_routing = subscription("b", "Beta");
        with_routing.provider.routing = Some(ProviderRouting {
            name: "Pro".to_string(),
            enabled: true,
            rules: vec![
                demo_rule(RuleAction::Proxy, &["x"], &[], &[]),
                demo_rule(RuleAction::Direct, &[], &["y"], &[]),
            ],
            ..ProviderRouting::default()
        });
        with_routing.options.ignore_provider_routing = true;
        state.subscriptions.push(with_routing);

        let items = provider_items(&state);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].id.as_str(), "b");
        assert_eq!(items[0].name.as_str(), "Pro");
        // The profile is ignored, so no rules are effective.
        assert_eq!(items[0].rules_text.as_str(), "0 правил");
        assert!(items[0].enabled);
        assert!(items[0].ignored);
    }

    #[test]
    fn provider_items_report_effective_rule_count() {
        let mut state = AppState::default();
        let mut sub = subscription("c", "Gamma");
        let mut disabled = demo_rule(RuleAction::Proxy, &["z"], &[], &[]);
        disabled.enabled = false;
        sub.provider.routing = Some(ProviderRouting {
            enabled: true,
            rules: vec![demo_rule(RuleAction::Proxy, &["x"], &[], &[]), disabled],
            ..ProviderRouting::default()
        });
        state.subscriptions.push(sub);
        let items = provider_items(&state);
        assert_eq!(items[0].rules_text.as_str(), "1 правило");
    }

    #[test]
    fn provider_items_fall_back_to_subscription_name() {
        let mut state = AppState::default();
        let mut sub = subscription("c", "Fallback");
        sub.provider.routing = Some(ProviderRouting {
            name: "   ".to_string(),
            enabled: false,
            ..ProviderRouting::default()
        });
        state.subscriptions.push(sub);
        let items = provider_items(&state);
        assert_eq!(items[0].name.as_str(), "Fallback");
        assert_eq!(items[0].rules_text.as_str(), "0 правил");
        assert!(!items[0].enabled);
    }

    #[test]
    fn provider_items_empty_without_routing() {
        let mut state = AppState::default();
        state.subscriptions.push(subscription("a", "Alpha"));
        assert!(provider_items(&state).is_empty());
    }
}

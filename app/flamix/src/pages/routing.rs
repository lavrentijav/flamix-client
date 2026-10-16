//! "Маршрутизация" page (page 2). OWNER: T17C (docs/tasks/T17-pages.md).
//!
//! Rust talks to the page only through the [`RoutingBridge`] global. Every change is written
//! through [`Controller::mutate`] and a reconnect (if a core is running) is scheduled with a
//! 700 ms debounce. All non-trivial logic is pure and unit-tested below.

use crate::controller::Controller;
use crate::{AppWindow, ProviderRoutingItem, RoutingBridge, RoutingRuleItem};
use flamix_core::model::{AppState, Routing, RoutingPreset, RoutingRule, RuleAction};
use slint::ComponentHandle;
use std::cell::Cell;
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

/// Split a comma-separated UI value into trimmed, non-empty entries.
fn split_values(text: &str) -> Vec<String> {
    text.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

fn join_values(rule: &RoutingRule, kind: usize) -> String {
    let list = match kind {
        1 => &rule.ips,
        2 => &rule.processes,
        _ => &rule.domains,
    };
    list.join(", ")
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

fn move_rule(rules: &mut [RoutingRule], index: i32, delta: i32) {
    let Ok(from) = usize::try_from(index) else {
        return;
    };
    let to = from as i64 + i64::from(delta);
    if to < 0 || to >= rules.len() as i64 {
        return;
    }
    rules.swap(from, to as usize);
}

fn rule_item(index: usize, rule: &RoutingRule) -> RoutingRuleItem {
    let kind = kind_index(rule);
    RoutingRuleItem {
        index: index as i32,
        action: action_index(rule.action),
        kind: kind as i32,
        values: join_values(rule, kind).into(),
        enabled: rule.enabled,
    }
}

fn build_rule_items(routing: &Routing) -> Vec<RoutingRuleItem> {
    routing
        .custom_rules
        .iter()
        .enumerate()
        .map(|(index, rule)| rule_item(index, rule))
        .collect()
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
                rules_text: rules_label(routing.rules.len()).into(),
                enabled: routing.enabled,
                ignored: sub.options.ignore_provider_routing,
            })
        })
        .collect()
}

// --- bridge publishing ----------------------------------------------------

fn publish(ui: &AppWindow, routing: &Routing, providers: Vec<ProviderRoutingItem>, error: String) {
    let bridge = ui.global::<RoutingBridge>();
    bridge.set_preset(preset_index(routing.preset));
    bridge.set_domain_strategy(strategy_index(&routing.domain_strategy));
    bridge.set_block_quic(routing.block_quic);
    bridge.set_rules(crate::view::model(build_rule_items(routing)));
    bridge.set_has_providers(!providers.is_empty());
    bridge.set_providers(crate::view::model(providers));
    bridge.set_error_text(error.into());
}

fn refresh(ui: &AppWindow, ctrl: &Controller) {
    let (routing, providers) = ctrl.with_state(|s| (s.settings.routing.clone(), provider_items(s)));
    let error = validation_error(&routing);
    publish(ui, &routing, providers, error);
}

fn repaint(weak: &slint::Weak<AppWindow>, ctrl: &Controller) {
    if let Some(ui) = weak.upgrade() {
        refresh(&ui, ctrl);
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
    refresh(ui, &ctrl);
    let defer = Defer::new(Arc::clone(&ctrl));

    {
        let weak = ui.as_weak();
        let ctrl = Arc::clone(&ctrl);
        let defer = defer.clone();
        ui.global::<RoutingBridge>()
            .on_preset_selected(move |index| {
                ctrl.mutate(|s| s.settings.routing.preset = preset_from_index(index));
                repaint(&weak, &ctrl);
                defer.schedule();
            });
    }
    {
        let weak = ui.as_weak();
        let ctrl = Arc::clone(&ctrl);
        let defer = defer.clone();
        ui.global::<RoutingBridge>()
            .on_rule_action_changed(move |index, action| {
                ctrl.mutate(|s| {
                    if let Some(rule) = rule_mut(&mut s.settings.routing, index) {
                        rule.action = action_from_index(action);
                    }
                });
                repaint(&weak, &ctrl);
                defer.schedule();
            });
    }
    {
        let ctrl = Arc::clone(&ctrl);
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
                // No refresh: keep the chips on the chosen kind so the next edit lands in it.
                defer.schedule();
            });
    }
    {
        let ctrl = Arc::clone(&ctrl);
        let defer = defer.clone();
        ui.global::<RoutingBridge>()
            .on_rule_values_changed(move |index, kind, text| {
                let kind = kind_from_index(kind).unwrap_or(0);
                ctrl.mutate(|s| {
                    if let Some(rule) = rule_mut(&mut s.settings.routing, index) {
                        set_rule_values(rule, kind, text.as_str());
                    }
                });
                // No refresh: preserve the LineEdit cursor while typing.
                defer.schedule();
            });
    }
    {
        let weak = ui.as_weak();
        let ctrl = Arc::clone(&ctrl);
        let defer = defer.clone();
        ui.global::<RoutingBridge>()
            .on_rule_enabled_changed(move |index, enabled| {
                ctrl.mutate(|s| {
                    if let Some(rule) = rule_mut(&mut s.settings.routing, index) {
                        rule.enabled = enabled;
                    }
                });
                repaint(&weak, &ctrl);
                defer.schedule();
            });
    }
    {
        let weak = ui.as_weak();
        let ctrl = Arc::clone(&ctrl);
        let defer = defer.clone();
        ui.global::<RoutingBridge>()
            .on_rule_move(move |index, delta| {
                ctrl.mutate(|s| move_rule(&mut s.settings.routing.custom_rules, index, delta));
                repaint(&weak, &ctrl);
                defer.schedule();
            });
    }
    {
        let weak = ui.as_weak();
        let ctrl = Arc::clone(&ctrl);
        let defer = defer.clone();
        ui.global::<RoutingBridge>().on_rule_delete(move |index| {
            ctrl.mutate(|s| {
                if let Ok(index) = usize::try_from(index) {
                    if index < s.settings.routing.custom_rules.len() {
                        s.settings.routing.custom_rules.remove(index);
                    }
                }
            });
            repaint(&weak, &ctrl);
            defer.schedule();
        });
    }
    {
        let weak = ui.as_weak();
        let ctrl = Arc::clone(&ctrl);
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
            repaint(&weak, &ctrl);
            defer.schedule();
        });
    }
    {
        let weak = ui.as_weak();
        let ctrl = Arc::clone(&ctrl);
        let defer = defer.clone();
        ui.global::<RoutingBridge>()
            .on_domain_strategy_changed(move |label| {
                let Some(strategy) = strategy_from_label(label.as_str()) else {
                    return;
                };
                ctrl.mutate(|s| s.settings.routing.domain_strategy = strategy.to_string());
                repaint(&weak, &ctrl);
                defer.schedule();
            });
    }
    {
        let weak = ui.as_weak();
        let ctrl = Arc::clone(&ctrl);
        let defer = defer.clone();
        ui.global::<RoutingBridge>()
            .on_block_quic_changed(move |enabled| {
                ctrl.mutate(|s| s.settings.routing.block_quic = enabled);
                repaint(&weak, &ctrl);
                defer.schedule();
            });
    }
    {
        let weak = ui.as_weak();
        let ctrl = Arc::clone(&ctrl);
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
                repaint(&weak, &ctrl);
                defer.schedule();
            });
    }
    {
        let weak = ui.as_weak();
        let ctrl = Arc::clone(&ctrl);
        let defer = defer.clone();
        ui.global::<RoutingBridge>()
            .on_provider_ignored_changed(move |id, ignored| {
                ctrl.mutate(|s| {
                    if let Some(sub) = subscription_mut(s, id.as_str()) {
                        sub.options.ignore_provider_routing = ignored;
                    }
                });
                repaint(&weak, &ctrl);
                defer.schedule();
            });
    }
}

fn subscription_mut<'a>(
    state: &'a mut AppState,
    id: &str,
) -> Option<&'a mut flamix_core::model::Subscription> {
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
    publish(ui, &demo_routing(), demo_providers(), String::new());
}

#[cfg(test)]
mod tests {
    use super::*;
    use flamix_core::model::{ProviderRouting, Subscription, SubscriptionOptions};

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
        assert_eq!(join_values(&rule, 1), "geoip:ru, geoip:cn");
        assert_eq!(join_values(&rule, 0), "");
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
        // Switching to the same kind is a no-op.
        change_rule_kind(&mut rule, 1);
        assert_eq!(rule.ips, vec!["a.com", "b.com"]);
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
        move_rule(&mut rules, 0, 1);
        assert_eq!(rules[0].domains, vec!["b"]);
        assert_eq!(rules[1].domains, vec!["a"]);
        // Out of bounds is ignored.
        move_rule(&mut rules, 0, -1);
        move_rule(&mut rules, 2, 1);
        assert_eq!(rules[0].domains, vec!["b"]);
        assert_eq!(rules[2].domains, vec!["c"]);
    }

    #[test]
    fn rule_item_reflects_model() {
        let rule = demo_rule(RuleAction::Block, &["geosite:ads"], &[], &[]);
        let item = rule_item(3, &rule);
        assert_eq!(item.index, 3);
        assert_eq!(item.action, 2);
        assert_eq!(item.kind, 0);
        assert_eq!(item.values.as_str(), "geosite:ads");
        assert!(item.enabled);
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
        assert_eq!(items[0].rules_text.as_str(), "2 правила");
        assert!(items[0].enabled);
        assert!(items[0].ignored);
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

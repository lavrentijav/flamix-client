//! "Маршрутизация" page (page 2). OWNER: T17C (docs/tasks/T17-pages.md), T28 value fields.
//!
//! Rust talks to the page only through the [`RoutingBridge`] global. Every change is written
//! through [`Controller::mutate`] and a reconnect (if a core is running) is scheduled with a
//! 700 ms debounce. All non-trivial logic is pure and unit-tested below.
//!
//! Each rule owns three editable lists (domains / IPs / processes). The lists live in
//! [`ListCache`] as *rows*: the stored entries plus one trailing empty input row, which may also
//! contain transient empty rows while the user adds an entry in the middle. The stored config
//! only ever sees the trimmed, non-empty entries.

use crate::controller::Controller;
use crate::{AppWindow, ProviderRoutingItem, RoutingBridge, RoutingRuleItem};
use flamix_core::model::{AppState, Routing, RoutingPreset, RoutingRule, RuleAction, Subscription};
use slint::{ComponentHandle, SharedString};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

/// Delay before applying a routing change to a running core.
const RECONNECT_DEBOUNCE: Duration = Duration::from_millis(700);

/// Domain strategy labels, used for the ComboBox mapping.
const STRATEGIES: [&str; 3] = ["AsIs", "IPIfNonMatch", "IPOnDemand"];

/// Number of editable value lists per rule: domains, IPs, processes.
const FIELD_COUNT: usize = 3;

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

/// Field index (0 domains, 1 IPs, 2 processes) as a `usize`, rejecting anything else.
fn field_index(field: i32) -> Option<usize> {
    usize::try_from(field).ok().filter(|f| *f < FIELD_COUNT)
}

/// The stored value list of a rule for a given field.
fn field_list(rule: &RoutingRule, field: usize) -> &Vec<String> {
    match field {
        1 => &rule.ips,
        2 => &rule.processes,
        _ => &rule.domains,
    }
}

/// Mutable access to the stored value list of a rule for a given field.
fn field_list_mut(rule: &mut RoutingRule, field: usize) -> &mut Vec<String> {
    match field {
        1 => &mut rule.ips,
        2 => &mut rule.processes,
        _ => &mut rule.domains,
    }
}

/// Insert `value` at `row`, clamped to the end of the list.
fn list_insert(rows: &mut Vec<String>, row: usize, value: &str) {
    let at = row.min(rows.len());
    rows.insert(at, value.to_string());
}

/// Remove the row at `row`; returns whether a row was removed. The last remaining row is kept.
fn list_remove(rows: &mut Vec<String>, row: usize) -> bool {
    if rows.len() <= 1 || row >= rows.len() {
        return false;
    }
    rows.remove(row);
    true
}

/// The ordered, trimmed, non-empty entries of an editable row list.
fn list_values(rows: &[String]) -> Vec<String> {
    rows.iter()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

/// Keep exactly one trailing empty input row. Empty rows in the middle are kept so the user can
/// still type into a row they just inserted.
fn normalize_rows(rows: &mut Vec<String>) {
    if rows.is_empty() {
        rows.push(String::new());
        return;
    }
    while rows.len() >= 2 && rows[rows.len() - 1].is_empty() && rows[rows.len() - 2].is_empty() {
        rows.pop();
    }
    if !rows[rows.len() - 1].is_empty() {
        rows.push(String::new());
    }
}

/// Editable rows for a stored list: the entries plus one trailing empty input row.
fn list_rows(list: &[String]) -> Vec<String> {
    let mut rows = list.to_vec();
    rows.push(String::new());
    rows
}

fn string_model(list: &[String]) -> slint::ModelRc<SharedString> {
    crate::view::model(
        list.iter()
            .map(|s| SharedString::from(s.as_str()))
            .collect(),
    )
}

fn is_empty_rule(rule: &RoutingRule) -> bool {
    rule.domains.is_empty() && rule.ips.is_empty() && rule.processes.is_empty()
}

/// First validation problem of the custom rules ("" when valid).
fn validation_error(routing: &Routing) -> String {
    for (index, rule) in routing.custom_rules.iter().enumerate() {
        if rule.enabled && is_empty_rule(rule) {
            return format!("Правило {}: хотя бы одно значение", index + 1);
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

/// Editable state of one rule that the model cannot store: the three row lists and the
/// UI-local collapsed flag.
struct RuleUi {
    rows: [Vec<String>; FIELD_COUNT],
    collapsed: bool,
}

impl RuleUi {
    fn from_rule(rule: &RoutingRule) -> Self {
        Self {
            rows: [
                list_rows(&rule.domains),
                list_rows(&rule.ips),
                list_rows(&rule.processes),
            ],
            collapsed: false,
        }
    }
}

/// Per-rule editable rows, kept in sync with the model by index. The UI edits here first and the
/// stored config is derived from these rows, so an in-progress empty row survives repaints.
#[derive(Default)]
struct ListCache {
    rules: Vec<RuleUi>,
    /// Coordinates of the row that should receive focus on the next repaint.
    pending_focus: Option<(i32, i32, i32)>,
    /// Monotonic focus request token; 0 means "no pending request" (consumed by the UI).
    focus_token: i32,
}

impl ListCache {
    /// Align the cache with the model: keep existing rows, add slots for new rules, drop removed
    /// trailing ones. Middle deletions/moves are applied explicitly by the handlers.
    fn sync(&mut self, routing: &Routing) {
        let count = routing.custom_rules.len();
        self.rules.truncate(count);
        while self.rules.len() < count {
            let rule = &routing.custom_rules[self.rules.len()];
            self.rules.push(RuleUi::from_rule(rule));
        }
    }

    fn rule_ui_mut(&mut self, index: i32) -> Option<&mut RuleUi> {
        usize::try_from(index)
            .ok()
            .and_then(|i| self.rules.get_mut(i))
    }

    /// Update the text of one row (raw; empty rows are tolerated while editing).
    fn edit_row(&mut self, index: i32, field: i32, row: i32, text: &str) {
        let Some(field) = field_index(field) else {
            return;
        };
        let Some(ui) = self.rule_ui_mut(index) else {
            return;
        };
        let rows = &mut ui.rows[field];
        let Some(row) = usize::try_from(row).ok().filter(|r| *r < rows.len()) else {
            return;
        };
        rows[row] = text.to_string();
        normalize_rows(rows);
    }

    /// Insert an empty row after `row`; returns the `(field, row)` to focus.
    fn insert_row(&mut self, index: i32, field: i32, row: i32) -> Option<(i32, i32)> {
        let field = field_index(field)?;
        let row = usize::try_from(row).ok()?;
        let ui = self.rule_ui_mut(index)?;
        let rows = &mut ui.rows[field];
        let at = row.saturating_add(1).min(rows.len());
        list_insert(rows, at, "");
        normalize_rows(rows);
        Some((field as i32, at.min(rows.len().saturating_sub(1)) as i32))
    }

    /// Remove an empty row; returns the `(field, row)` to focus (the row above).
    fn remove_row(&mut self, index: i32, field: i32, row: i32) -> Option<(i32, i32)> {
        let field = field_index(field)?;
        let row = usize::try_from(row).ok()?;
        let ui = self.rule_ui_mut(index)?;
        let rows = &mut ui.rows[field];
        if row >= rows.len() || !rows[row].is_empty() {
            return None;
        }
        if !list_remove(rows, row) {
            // Only the first row is left: clearing it is a no-op for the model.
            rows[row].clear();
            return None;
        }
        normalize_rows(rows);
        Some((
            field as i32,
            row.saturating_sub(1).min(rows.len().saturating_sub(1)) as i32,
        ))
    }

    fn set_collapsed(&mut self, index: i32, collapsed: bool) {
        if let Some(ui) = self.rule_ui_mut(index) {
            ui.collapsed = collapsed;
        }
    }

    /// Write the trimmed, non-empty rows of one rule back into the stored config.
    fn write_config(&self, routing: &mut Routing, index: i32) {
        let Some(ui) = usize::try_from(index).ok().and_then(|i| self.rules.get(i)) else {
            return;
        };
        let Some(rule) = rule_mut(routing, index) else {
            return;
        };
        for (field, rows) in ui.rows.iter().enumerate() {
            *field_list_mut(rule, field) = list_values(rows);
        }
    }

    /// Mirror a reorder of the model (same bounds rules as [`move_rule`]).
    fn swap(&mut self, index: i32, delta: i32) {
        let Ok(from) = usize::try_from(index) else {
            return;
        };
        if from >= self.rules.len() {
            return;
        }
        let to = from as i64 + i64::from(delta);
        if to < 0 || to >= self.rules.len() as i64 {
            return;
        }
        self.rules.swap(from, to as usize);
    }

    fn remove(&mut self, index: i32) {
        if let Some(index) = usize::try_from(index)
            .ok()
            .filter(|index| *index < self.rules.len())
        {
            self.rules.remove(index);
        }
    }

    fn request_focus(&mut self, rule: i32, field: i32, row: i32) {
        self.pending_focus = Some((rule, field, row));
    }

    /// Take the pending focus request and stamp it with a fresh non-zero token.
    fn take_focus(&mut self) -> Option<(i32, i32, i32, i32)> {
        let (rule, field, row) = self.pending_focus.take()?;
        self.focus_token = self.focus_token.wrapping_add(1);
        if self.focus_token <= 0 {
            self.focus_token = 1;
        }
        Some((rule, field, row, self.focus_token))
    }

    /// Build the UI items, falling back to the model when a slot is missing.
    fn build_items(&self, routing: &Routing) -> Vec<RoutingRuleItem> {
        routing
            .custom_rules
            .iter()
            .enumerate()
            .map(|(index, rule)| {
                let ui = self.rules.get(index);
                let rows = |field: usize| match ui {
                    Some(ui) => string_model(&ui.rows[field]),
                    None => string_model(&list_rows(field_list(rule, field))),
                };
                RoutingRuleItem {
                    index: index as i32,
                    action: action_index(rule.action),
                    enabled: rule.enabled,
                    collapsed: ui.map(|ui| ui.collapsed).unwrap_or(false),
                    domains: rows(0),
                    ips: rows(1),
                    processes: rows(2),
                }
            })
            .collect()
    }
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
    focus: Option<(i32, i32, i32, i32)>,
) {
    let bridge = ui.global::<RoutingBridge>();
    bridge.set_preset(preset_index(routing.preset));
    bridge.set_domain_strategy(strategy_index(&routing.domain_strategy));
    bridge.set_block_quic(routing.block_quic);
    bridge.set_error_text(error.into());
    if let Some((rule, field, row, token)) = focus {
        bridge.set_focus_rule(rule);
        bridge.set_focus_field(field);
        bridge.set_focus_row(row);
        bridge.set_focus_token(token);
    }
    // The rules model is published last: the freshly created value rows read the focus target
    // above during their `init` and focus themselves.
    bridge.set_rules(crate::view::model(rules));
    bridge.set_has_providers(!providers.is_empty());
    bridge.set_providers(crate::view::model(providers));
}

fn refresh(ui: &AppWindow, ctrl: &Controller, cache: &Rc<RefCell<ListCache>>) {
    let (routing, providers) = ctrl.with_state(|s| (s.settings.routing.clone(), provider_items(s)));
    let error = validation_error(&routing);
    let (rules, focus) = {
        let mut cache = cache.borrow_mut();
        cache.sync(&routing);
        (cache.build_items(&routing), cache.take_focus())
    };
    publish(ui, &routing, rules, providers, error, focus);
}

fn repaint(weak: &slint::Weak<AppWindow>, ctrl: &Controller, cache: &Rc<RefCell<ListCache>>) {
    if let Some(ui) = weak.upgrade() {
        refresh(&ui, ctrl, cache);
    }
}

/// Repaint on the next event-loop iteration. Structural list edits come from key handlers, so the
/// model is rebuilt after the key event finished instead of destroying the focused row mid-event.
fn repaint_later(
    weak: slint::Weak<AppWindow>,
    ctrl: Arc<Controller>,
    cache: Rc<RefCell<ListCache>>,
) {
    slint::Timer::single_shot(Duration::ZERO, move || {
        repaint(&weak, &ctrl, &cache);
    });
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
    let cache: Rc<RefCell<ListCache>> = Rc::new(RefCell::new(ListCache::default()));
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
            .on_rule_collapsed_changed(move |index, collapsed| {
                cache.borrow_mut().set_collapsed(index, collapsed);
                repaint(&weak, &ctrl, &cache);
                defer.schedule();
            });
    }
    {
        let ctrl = Arc::clone(&ctrl);
        let cache = Rc::clone(&cache);
        let defer = defer.clone();
        ui.global::<RoutingBridge>()
            .on_rule_item_edited(move |index, field, row, text| {
                cache
                    .borrow_mut()
                    .edit_row(index, field, row, text.as_str());
                ctrl.mutate(|s| cache.borrow().write_config(&mut s.settings.routing, index));
                // No refresh: keep the LineEdit cursor where it is while typing.
                defer.schedule();
            });
    }
    {
        let weak = ui.as_weak();
        let ctrl = Arc::clone(&ctrl);
        let cache = Rc::clone(&cache);
        let defer = defer.clone();
        ui.global::<RoutingBridge>()
            .on_rule_item_insert(move |index, field, row| {
                let Some((focus_field, focus_row)) =
                    cache.borrow_mut().insert_row(index, field, row)
                else {
                    return;
                };
                ctrl.mutate(|s| cache.borrow().write_config(&mut s.settings.routing, index));
                cache
                    .borrow_mut()
                    .request_focus(index, focus_field, focus_row);
                repaint_later(weak.clone(), Arc::clone(&ctrl), Rc::clone(&cache));
                defer.schedule();
            });
    }
    {
        let weak = ui.as_weak();
        let ctrl = Arc::clone(&ctrl);
        let cache = Rc::clone(&cache);
        let defer = defer.clone();
        ui.global::<RoutingBridge>()
            .on_rule_item_remove(move |index, field, row| {
                let Some((focus_field, focus_row)) =
                    cache.borrow_mut().remove_row(index, field, row)
                else {
                    return;
                };
                ctrl.mutate(|s| cache.borrow().write_config(&mut s.settings.routing, index));
                cache
                    .borrow_mut()
                    .request_focus(index, focus_field, focus_row);
                repaint_later(weak.clone(), Arc::clone(&ctrl), Rc::clone(&cache));
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
                &["domain:openai.com", "geosite:google", "full:chatgpt.com"],
                &["geoip:us"],
                &[],
            ),
            demo_rule(
                RuleAction::Direct,
                &["domain:gosuslugi.ru"],
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
    let mut cache = ListCache::default();
    cache.sync(&routing);
    let rules = cache.build_items(&routing);
    publish(ui, &routing, rules, demo_providers(), String::new(), None);
}

#[cfg(test)]
mod tests {
    use super::*;
    use flamix_core::model::{ProviderRouting, SubscriptionOptions};
    use slint::Model;

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

    fn item_strings(model: &slint::ModelRc<SharedString>) -> Vec<String> {
        (0..model.row_count())
            .filter_map(|i| model.row_data(i).map(|s| s.to_string()))
            .collect()
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
    fn list_insert_clamps_to_the_end() {
        let mut rows = vec!["a".to_string(), "b".to_string()];
        list_insert(&mut rows, 1, "x");
        assert_eq!(rows, vec!["a", "x", "b"]);
        list_insert(&mut rows, 99, "end");
        assert_eq!(rows, vec!["a", "x", "b", "end"]);
    }

    #[test]
    fn list_remove_keeps_the_last_row() {
        let mut rows = vec!["a".to_string(), "b".to_string()];
        assert!(list_remove(&mut rows, 0));
        assert_eq!(rows, vec!["b"]);
        assert!(!list_remove(&mut rows, 0));
        assert_eq!(rows, vec!["b"]);
        assert!(!list_remove(&mut rows, 5));
    }

    #[test]
    fn list_values_trims_and_drops_empty() {
        let rows = vec![
            "  geosite:ru  ".to_string(),
            String::new(),
            "10.0.0.0/8".to_string(),
            "   ".to_string(),
        ];
        assert_eq!(list_values(&rows), vec!["geosite:ru", "10.0.0.0/8"]);
    }

    #[test]
    fn normalize_keeps_one_trailing_row_and_middle_empties() {
        let mut rows = vec!["a".to_string()];
        normalize_rows(&mut rows);
        assert_eq!(rows, vec!["a", ""]);

        let mut rows = vec!["a".to_string(), "".to_string(), "".to_string()];
        normalize_rows(&mut rows);
        assert_eq!(rows, vec!["a", ""]);

        let mut rows = vec!["a".to_string(), "".to_string(), "b".to_string()];
        normalize_rows(&mut rows);
        assert_eq!(rows, vec!["a", "", "b", ""]);

        let mut rows: Vec<String> = Vec::new();
        normalize_rows(&mut rows);
        assert_eq!(rows, vec![""]);
    }

    #[test]
    fn list_rows_appends_an_input_row() {
        let rows = list_rows(&["a".to_string(), "b".to_string()]);
        assert_eq!(rows, vec!["a", "b", ""]);
        assert_eq!(list_rows(&[]), vec![""]);
    }

    #[test]
    fn field_mapping_roundtrips() {
        let mut rule = demo_rule(RuleAction::Proxy, &["d"], &["i"], &["p"]);
        assert_eq!(field_list(&rule, 0), &vec!["d".to_string()]);
        assert_eq!(field_list(&rule, 1), &vec!["i".to_string()]);
        assert_eq!(field_list(&rule, 2), &vec!["p".to_string()]);
        field_list_mut(&mut rule, 1).push("i2".to_string());
        assert_eq!(rule.ips, vec!["i", "i2"]);
        assert_eq!(field_index(2), Some(2));
        assert_eq!(field_index(3), None);
        assert_eq!(field_index(-1), None);
    }

    #[test]
    fn edit_row_updates_and_normalizes() {
        let mut cache = ListCache::default();
        cache.sync(&Routing {
            custom_rules: vec![demo_rule(RuleAction::Proxy, &[], &[], &[])],
            ..Routing::default()
        });
        // Typing into the only (input) row appends a fresh trailing row.
        cache.edit_row(0, 0, 0, "a.com");
        assert_eq!(cache.rules[0].rows[0], vec!["a.com", ""]);
        // A trailing space is stored raw but ignored by the model writer.
        cache.edit_row(0, 0, 1, "   ");
        assert_eq!(cache.rules[0].rows[0], vec!["a.com", "   ", ""]);
        // Out-of-range coordinates are ignored.
        cache.edit_row(0, 0, 9, "x");
        cache.edit_row(0, 5, 0, "x");
        cache.edit_row(9, 0, 0, "x");
        assert_eq!(cache.rules[0].rows[0], vec!["a.com", "   ", ""]);
    }

    #[test]
    fn insert_row_inserts_below_and_reports_focus() {
        let mut cache = ListCache::default();
        cache.sync(&Routing {
            custom_rules: vec![demo_rule(RuleAction::Proxy, &["a", "b"], &[], &[])],
            ..Routing::default()
        });
        assert_eq!(cache.rules[0].rows[0], vec!["a", "b", ""]);
        // Enter on the first row inserts a new row right below it.
        assert_eq!(cache.insert_row(0, 0, 0), Some((0, 1)));
        assert_eq!(cache.rules[0].rows[0], vec!["a", "", "b", ""]);
        // Enter on the trailing input row stays at the end.
        assert_eq!(cache.insert_row(0, 0, 3), Some((0, 3)));
        assert_eq!(cache.rules[0].rows[0], vec!["a", "", "b", ""]);
        // Invalid coordinates do nothing.
        assert_eq!(cache.insert_row(0, 7, 0), None);
        assert_eq!(cache.insert_row(4, 0, 0), None);
    }

    #[test]
    fn remove_row_drops_empty_and_focuses_above() {
        let mut cache = ListCache::default();
        cache.sync(&Routing {
            custom_rules: vec![demo_rule(RuleAction::Proxy, &["a", "b"], &[], &[])],
            ..Routing::default()
        });
        // rows == ["a", "b", ""]
        assert_eq!(cache.remove_row(0, 0, 2), Some((0, 1)));
        assert_eq!(cache.rules[0].rows[0], vec!["a", "b", ""]);
        // A non-empty row is not removed (the LineEdit edits it instead).
        assert_eq!(cache.remove_row(0, 0, 0), None);
        assert_eq!(cache.rules[0].rows[0], vec!["a", "b", ""]);
        // Removing the only input row of an empty field keeps one row.
        let mut cache = ListCache::default();
        cache.sync(&Routing {
            custom_rules: vec![demo_rule(RuleAction::Proxy, &[], &[], &[])],
            ..Routing::default()
        });
        assert_eq!(cache.remove_row(0, 0, 0), None);
        assert_eq!(cache.rules[0].rows[0], vec![""]);
    }

    #[test]
    fn cache_sync_tracks_rule_count() {
        let mut cache = ListCache::default();
        let mut routing = Routing::default();
        routing
            .custom_rules
            .push(demo_rule(RuleAction::Proxy, &["a"], &[], &[]));
        cache.sync(&routing);
        assert_eq!(cache.rules.len(), 1);
        routing
            .custom_rules
            .push(demo_rule(RuleAction::Direct, &[], &["i"], &[]));
        cache.sync(&routing);
        assert_eq!(cache.rules.len(), 2);
        assert_eq!(cache.rules[1].rows[0], vec![""]);
        assert_eq!(cache.rules[1].rows[1], vec!["i", ""]);
        routing.custom_rules.clear();
        cache.sync(&routing);
        assert!(cache.rules.is_empty());
    }

    #[test]
    fn cache_swap_and_remove_mirror_reorders() {
        let mut cache = ListCache::default();
        cache.sync(&Routing {
            custom_rules: vec![
                demo_rule(RuleAction::Proxy, &["a"], &[], &[]),
                demo_rule(RuleAction::Proxy, &["b"], &[], &[]),
            ],
            ..Routing::default()
        });
        cache.swap(0, 1);
        assert_eq!(cache.rules[0].rows[0], vec!["b", ""]);
        assert_eq!(cache.rules[1].rows[0], vec!["a", ""]);
        cache.swap(0, -1);
        assert_eq!(cache.rules[0].rows[0], vec!["b", ""]);
        cache.remove(0);
        assert_eq!(cache.rules.len(), 1);
        assert_eq!(cache.rules[0].rows[0], vec!["a", ""]);
        cache.remove(5);
        assert_eq!(cache.rules.len(), 1);
    }

    #[test]
    fn write_config_uses_only_non_empty_rows() {
        let mut cache = ListCache::default();
        cache.sync(&Routing {
            custom_rules: vec![demo_rule(RuleAction::Proxy, &[], &[], &[])],
            ..Routing::default()
        });
        cache.edit_row(0, 0, 0, " domain:a.com ");
        cache.edit_row(0, 1, 0, "10.0.0.0/8");
        cache.edit_row(0, 2, 0, "telegram.exe");
        let mut routing = Routing {
            custom_rules: vec![demo_rule(RuleAction::Proxy, &["old"], &[], &[])],
            ..Routing::default()
        };
        cache.write_config(&mut routing, 0);
        assert_eq!(routing.custom_rules[0].domains, vec!["domain:a.com"]);
        assert_eq!(routing.custom_rules[0].ips, vec!["10.0.0.0/8"]);
        assert_eq!(routing.custom_rules[0].processes, vec!["telegram.exe"]);
        // Unknown index is ignored.
        cache.write_config(&mut routing, 5);
    }

    #[test]
    fn take_focus_stamps_a_fresh_token() {
        let mut cache = ListCache::default();
        assert_eq!(cache.take_focus(), None);
        cache.request_focus(1, 2, 3);
        assert_eq!(cache.take_focus(), Some((1, 2, 3, 1)));
        assert_eq!(cache.take_focus(), None);
        cache.request_focus(0, 0, 0);
        assert_eq!(cache.take_focus(), Some((0, 0, 0, 2)));
    }

    #[test]
    fn build_items_maps_rows_and_model() {
        let mut cache = ListCache::default();
        let routing = Routing {
            custom_rules: vec![
                demo_rule(RuleAction::Block, &["geosite:ads"], &[], &[]),
                demo_rule(RuleAction::Direct, &[], &["geoip:ru"], &[]),
            ],
            ..Routing::default()
        };
        cache.sync(&routing);
        cache.set_collapsed(1, true);
        cache.edit_row(0, 0, 1, "full:x.y");
        let items = cache.build_items(&routing);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].index, 0);
        assert_eq!(items[0].action, 2);
        assert!(items[0].enabled);
        assert!(!items[0].collapsed);
        assert_eq!(
            item_strings(&items[0].domains),
            vec!["geosite:ads", "full:x.y", ""]
        );
        assert_eq!(item_strings(&items[0].ips), vec![""]);
        assert_eq!(item_strings(&items[0].processes), vec![""]);
        assert!(items[1].collapsed);
        assert_eq!(item_strings(&items[1].ips), vec!["geoip:ru", ""]);
    }

    #[test]
    fn build_items_falls_back_to_the_model_without_a_cache_slot() {
        let routing = Routing {
            custom_rules: vec![demo_rule(RuleAction::Proxy, &["a.com"], &[], &[])],
            ..Routing::default()
        };
        let items = ListCache::default().build_items(&routing);
        assert_eq!(item_strings(&items[0].domains), vec!["a.com", ""]);
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
        assert_eq!(
            validation_error(&routing),
            "Правило 1: хотя бы одно значение"
        );
        routing
            .custom_rules
            .push(demo_rule(RuleAction::Direct, &["a.com"], &[], &[]));
        assert_eq!(
            validation_error(&routing),
            "Правило 1: хотя бы одно значение"
        );
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

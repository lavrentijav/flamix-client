//! Android entry point for Flamix (M4: phone layout).
//!
//! Kept in its own crate so the Android build does not pull in the desktop-only
//! dependencies of `app/flamix` (tray, native file dialogs, clipboard, ...).
//!
//! The window is `AndroidWindow` from `ui/android.slint`. State lives in `state.json` in the app's
//! files dir (same format as the desktop app); see [`phone`] for the logic.

pub mod applog;
#[cfg(unix)]
pub mod bridge;
#[cfg(unix)]
pub mod external;
pub mod phone;
#[cfg(unix)]
pub mod tunnel;

slint::include_modules!();

use flamix_core::model::{AppState, MirrorMode, RoutingRule, RuleAction};
use flamix_core::store;
use slint::{ComponentHandle, ModelRc, SharedString, Timer, TimerMode, VecModel};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// VPN calls. On Android they go through the JNI bridge to Kotlin; elsewhere the phone UI
/// can still be shown but cannot connect.
#[cfg(unix)]
mod vpn {
    pub use crate::bridge::{connect, disconnect, is_running};
}

#[cfg(not(unix))]
mod vpn {
    pub fn connect(_config_json: &str) -> Result<(), String> {
        Err("VPN доступен только на Android".into())
    }
    pub fn disconnect() -> Result<(), String> {
        Ok(())
    }
    pub fn is_running() -> bool {
        false
    }
}

#[cfg(not(unix))]
mod external {
    pub fn open(_url: &str) -> Result<(), String> {
        Err("Открытие ссылок только на Android".into())
    }
    pub fn read_clipboard() -> Result<String, String> {
        Err("Буфер обмена доступен только на Android".into())
    }
}

/// UI-only state shared by the callbacks. Behind a mutex because refresh callbacks hand the
/// updating list over from the fetch thread.
#[derive(Default)]
struct UiState {
    /// Search text of the servers tab.
    filter: String,
    /// Subscription ids with a refresh in flight (the refresh button turns accent).
    updating: Vec<String>,
    /// Subscription the settings sheet is editing.
    sheet_id: String,
    /// Id of the current press on the version line (0 = none); a long press fires only while
    /// its id is still current.
    press: u64,
    press_seq: u64,
}

type Shared = Arc<Mutex<UiState>>;

fn lock(shared: &Shared) -> std::sync::MutexGuard<'_, UiState> {
    shared.lock().unwrap_or_else(|e| e.into_inner())
}

fn color(rgba: Option<[u8; 4]>) -> slint::Color {
    match rgba {
        Some([r, g, b, a]) => slint::Color::from_argb_u8(a, r, g, b),
        None => slint::Color::from_argb_u8(0, 0, 0, 0),
    }
}

fn server_item(row: phone::ServerRow, selected: Option<&str>) -> PhoneServer {
    let is_selected = selected == Some(row.id.as_str());
    let (has_quota, quota_fraction, quota_text) = match row.quota {
        Some((fraction, text)) => (true, fraction, text),
        None => (false, 0.0, String::new()),
    };
    PhoneServer {
        id: row.id.into(),
        name: row.name.into(),
        subtitle: row.subtitle.into(),
        address: row.address.into(),
        is_exit: row.is_exit,
        group: i32::try_from(row.group).unwrap_or(0),
        selected: is_selected,
        ping: row.ping,
        has_quota,
        quota_fraction,
        quota_text: quota_text.into(),
    }
}

fn sub_item(card: phone::SubCard, selected: Option<&str>) -> PhoneSub {
    let (has_traffic, traffic_text, traffic_fraction, expire_text) = match card.traffic {
        Some((text, fraction, expire)) => (true, text, fraction, expire),
        None => (false, String::new(), 0.0, String::new()),
    };
    let (banner_text, banner_tone, banner_color, banner_button, banner_link) = match card.banner {
        Some(b) => (b.text, b.tone, color(b.color), b.button_text, b.link),
        None => (
            String::new(),
            phone::TONE_INFO,
            color(None),
            String::new(),
            String::new(),
        ),
    };
    let servers: Vec<PhoneServer> = card
        .servers
        .into_iter()
        .map(|row| server_item(row, selected))
        .collect();
    PhoneSub {
        id: card.id.into(),
        name: card.name.into(),
        local: card.local,
        collapsed: card.collapsed,
        pinned: card.pinned,
        updating: card.updating,
        has_traffic,
        traffic_text: traffic_text.into(),
        traffic_fraction,
        expire_text: expire_text.into(),
        banner_text: banner_text.into(),
        banner_tone,
        banner_color,
        banner_button: banner_button.into(),
        banner_link: banner_link.into(),
        announce: card.announce.into(),
        error: card.error.into(),
        support_kind: card.support_kind,
        support_url: card.support_url.into(),
        web_url: card.web_url.into(),
        servers: ModelRc::new(VecModel::from(servers)),
    }
}

/// Custom rule at list position `index` (negative or out of range = none).
fn rule_at(state: &mut AppState, index: i32) -> Option<&mut RoutingRule> {
    let i = usize::try_from(index).ok()?;
    state.settings.routing.custom_rules.get_mut(i)
}

/// Pushes the statistics tab (today, 30 days, lifetime, columns, busiest servers).
fn refresh_stats(window: &AndroidWindow, state: &AppState) {
    let view = phone::stats_view(state, chrono::Local::now().date_naive());
    let cards: Vec<PhoneStatCard> = view
        .cards
        .into_iter()
        .map(|c| PhoneStatCard {
            title: c.title.into(),
            value: c.value.into(),
            up: c.up.into(),
            down: c.down.into(),
        })
        .collect();
    window.set_stat_cards(ModelRc::new(VecModel::from(cards)));
    let days: Vec<PhoneDay> = view
        .days
        .into_iter()
        .map(|d| PhoneDay {
            label: d.label.into(),
            up: d.up,
            down: d.down,
        })
        .collect();
    window.set_stat_days(ModelRc::new(VecModel::from(days)));
    let top: Vec<PhoneTop> = view
        .top
        .into_iter()
        .map(|t| PhoneTop {
            name: t.name.into(),
            iso: t.iso.into(),
            used: phone::human_bytes(t.used).into(),
            fraction: t.fraction,
        })
        .collect();
    window.set_stat_top(ModelRc::new(VecModel::from(top)));
}

/// Pushes the settings chips (core, theme, language) from `state`.
fn refresh_settings(window: &AndroidWindow, state: &AppState) {
    let core = phone::CORE_OVERRIDES
        .iter()
        .position(|c| *c == state.settings.core_override)
        .unwrap_or(0);
    window.set_core_index(i32::try_from(core).unwrap_or(0));
    window.set_theme_index(match state.settings.theme {
        flamix_core::model::Theme::System => 0,
        flamix_core::model::Theme::Dark => 1,
        flamix_core::model::Theme::Light => 2,
    });
    window.set_language_index(if state.settings.language.eq_ignore_ascii_case("en") {
        1
    } else {
        0
    });
}

/// Pushes the DNS fields and the ping URL (on load and after a save; the user edits them in place).
fn load_settings_fields(window: &AndroidWindow, state: &AppState) {
    let dns = &state.settings.dns;
    window.set_dns_remote(dns.remote.clone().into());
    window.set_dns_direct(dns.direct.clone().into());
    window.set_dns_bootstrap(dns.bootstrap.clone().into());
    window.set_ping_url(state.settings.ping_url.clone().into());
}

/// Pushes the routing tab (presets, Anixart mirror, rules, provider routing) from `state`.
fn refresh_routing(window: &AndroidWindow, state: &AppState) {
    let routing = &state.settings.routing;
    window.set_routing_preset(phone::preset_index(routing.preset));
    window.set_routing_mirror(match routing.anixart_mirror {
        MirrorMode::Direct => 0,
        MirrorMode::Proxy => 1,
    });
    let strategy = phone::DOMAIN_STRATEGIES
        .iter()
        .position(|s| *s == routing.domain_strategy)
        .unwrap_or(1);
    window.set_routing_strategy(i32::try_from(strategy).unwrap_or(1));
    window.set_routing_quic(routing.block_quic);
    let cards = phone::rule_cards(state);
    let count = cards.len();
    let rules: Vec<PhoneRule> = cards
        .into_iter()
        .map(|c| PhoneRule {
            index: i32::try_from(c.index).unwrap_or(0),
            action: c.action,
            enabled: c.enabled,
            kind: c.kind,
            domains: c.domains.into(),
            ips: c.ips.into(),
            processes: c.processes.into(),
        })
        .collect();
    window.set_rules(ModelRc::new(VecModel::from(rules)));
    window.set_rules_summary(phone::rules_label(count).into());
    window.set_routing_error(phone::routing_error(state).into());
    let providers: Vec<PhoneProvider> = phone::provider_cards(state)
        .into_iter()
        .map(|p| PhoneProvider {
            id: p.id.into(),
            name: p.name.into(),
            rules_text: p.rules_text.into(),
            enabled: p.enabled,
            ignored: p.ignored,
        })
        .collect();
    window.set_providers(ModelRc::new(VecModel::from(providers)));
}

/// Opens the device id editor with the current values.
fn open_device_id_editor(window: &AndroidWindow, dir: &Path) {
    let Ok(state) = store::load(dir) else { return };
    window.set_hwid_auto(state.device_id.clone().unwrap_or_default().into());
    window.set_hwid_input(state.hwid_override.clone().unwrap_or_default().into());
    window.set_hwid_error("".into());
    window.set_hwid_open(true);
}

/// Pushes the subscription cards and the selection from `state` into the window.
fn refresh(window: &AndroidWindow, state: &AppState, shared: &Shared) {
    refresh_stats(window, state);
    refresh_routing(window, state);
    refresh_settings(window, state);
    let (filter, updating) = {
        let ui = lock(shared);
        (ui.filter.clone(), ui.updating.clone())
    };
    let selected = state.selected_server.as_deref();
    let subs: Vec<PhoneSub> = phone::sub_cards(state, &updating, &filter)
        .into_iter()
        .map(|card| sub_item(card, selected))
        .collect();
    window.set_subs(ModelRc::new(VecModel::from(subs)));
    window.set_selected_name(phone::selected_name(state).into());
}

/// Reloads the state file and refreshes the window from it.
fn reload(window: &AndroidWindow, dir: &Path, shared: &Shared) {
    match store::load(dir) {
        Ok(state) => refresh(window, &state, shared),
        Err(err) => window.set_status_text(format!("Не удалось прочитать данные: {err}").into()),
    }
}

/// Fetches subscription `id` on a plain thread and stores the result (UI thread afterwards).
/// A refresh already in flight for the same id is not started twice.
fn start_refresh(window: &AndroidWindow, dir: &Path, shared: &Shared, id: String) {
    let Ok(state) = store::load(dir) else { return };
    let Some(sub) = state.subscriptions.iter().find(|s| s.id == id).cloned() else {
        return;
    };
    {
        let mut ui = lock(shared);
        if ui.updating.contains(&id) {
            return;
        }
        ui.updating.push(id.clone());
    }
    refresh(window, &state, shared);
    let hwid = state.effective_hwid();
    let weak = window.as_weak();
    let dir_thread = dir.to_path_buf();
    let shared_thread = shared.clone();
    let id_thread = id.clone();
    let spawned = std::thread::Builder::new()
        .name("subscription-refresh".into())
        .spawn(move || {
            let mut sub = sub;
            let fetched = phone::fetch_subscription(&mut sub, hwid);
            let _ = slint::invoke_from_event_loop(move || {
                lock(&shared_thread).updating.retain(|x| *x != id_thread);
                let Some(w) = weak.upgrade() else { return };
                let stored = phone::apply_fetched(&dir_thread, sub);
                match fetched {
                    Ok(()) => {
                        let text = "Подписка обновлена".to_string();
                        w.set_add_status(text.clone().into());
                        w.set_status_text(text.into());
                    }
                    Err(err) => {
                        // The error is kept on the subscription (last_error) and shown on its card.
                        let text = format!("Ошибка обновления: {err}");
                        w.set_add_status(text.clone().into());
                        w.set_status_text(text.into());
                    }
                }
                match stored {
                    Ok(state) => refresh(&w, &state, &shared_thread),
                    Err(err) => w.set_status_text(err.into()),
                }
            });
        });
    if let Err(err) = spawned {
        lock(shared).updating.retain(|x| *x != id);
        window.set_status_text(format!("Не удалось запустить обновление: {err}").into());
        reload(window, dir, shared);
    }
}

/// Imports pasted text and writes the result line to the add tab. Returns true on success.
fn import_and_report(window: &AndroidWindow, dir: &Path, shared: &Shared, text: &str) -> bool {
    match phone::import_text(dir, text) {
        Ok((outcome, state)) => {
            refresh(window, &state, shared);
            let message = match outcome {
                phone::Imported::Subscription { id, name } => {
                    let label = if name.is_empty() { id.clone() } else { name };
                    start_refresh(window, dir, shared, id);
                    format!("Подписка добавлена: {label}")
                }
                phone::Imported::Links { added, skipped } => phone::links_status(added, skipped),
                phone::Imported::Routing { enabled } => {
                    if enabled {
                        "Маршрутизация включена".into()
                    } else {
                        "Маршрутизация отключена".into()
                    }
                }
                phone::Imported::Unsupported => "Нет подписки для маршрутизации".into(),
            };
            window.set_add_status(message.into());
            true
        }
        Err(err) => {
            window.set_add_status(err.into());
            false
        }
    }
}

/// Shows the state returned by a change, or its error text.
fn show(window: &AndroidWindow, shared: &Shared, result: Result<AppState, String>) {
    match result {
        Ok(state) => {
            refresh(window, &state, shared);
            window.set_status_text("".into());
        }
        Err(err) => window.set_status_text(err.into()),
    }
}

/// Loads the state for the first screen and wires the callbacks. Blocks in the Slint event loop.
pub fn run(data_dir: PathBuf) -> Result<(), slint::PlatformError> {
    let window = AndroidWindow::new()?;
    let shared: Shared = Arc::new(Mutex::new(UiState::default()));
    reload(&window, &data_dir, &shared);
    if let Ok(state) = store::load(&data_dir) {
        load_settings_fields(&window, &state);
    }

    window.set_version_text(env!("CARGO_PKG_VERSION").into());
    window.set_log_text(applog::tail(&data_dir, 200).into());
    window.on_refresh_logs({
        let weak = window.as_weak();
        let dir = data_dir.clone();
        move || {
            if let Some(w) = weak.upgrade() {
                w.set_log_text(applog::tail(&dir, 200).into());
            }
        }
    });
    window.on_clear_logs({
        let weak = window.as_weak();
        let dir = data_dir.clone();
        move || {
            applog::clear(&dir);
            if let Some(w) = weak.upgrade() {
                w.set_log_text("".into());
            }
        }
    });

    window.on_select_server({
        let weak = window.as_weak();
        let dir = data_dir.clone();
        let shared = shared.clone();
        move |id: SharedString| {
            let Some(w) = weak.upgrade() else { return };
            let id = id.to_string();
            let result = phone::update_state(&dir, |st| {
                if st.server(&id).is_some() {
                    st.selected_server = Some(id.clone());
                }
            });
            show(&w, &shared, result);
        }
    });

    window.on_search_changed({
        let weak = window.as_weak();
        let dir = data_dir.clone();
        let shared = shared.clone();
        move |text: SharedString| {
            let Some(w) = weak.upgrade() else { return };
            lock(&shared).filter = text.to_string();
            reload(&w, &dir, &shared);
        }
    });

    window.on_sub_toggle({
        let weak = window.as_weak();
        let dir = data_dir.clone();
        let shared = shared.clone();
        move |id: SharedString| {
            let Some(w) = weak.upgrade() else { return };
            let result = phone::edit_sub(&dir, &id, |sub| sub.collapsed = !sub.collapsed);
            show(&w, &shared, result);
        }
    });

    window.on_sub_pin({
        let weak = window.as_weak();
        let dir = data_dir.clone();
        let shared = shared.clone();
        move |id: SharedString| {
            let Some(w) = weak.upgrade() else { return };
            let result = phone::edit_sub(&dir, &id, |sub| sub.pinned = !sub.pinned);
            show(&w, &shared, result);
        }
    });

    window.on_sub_refresh({
        let weak = window.as_weak();
        let dir = data_dir.clone();
        let shared = shared.clone();
        move |id: SharedString| {
            let Some(w) = weak.upgrade() else { return };
            start_refresh(&w, &dir, &shared, id.to_string());
        }
    });

    window.on_sub_open_settings({
        let weak = window.as_weak();
        let dir = data_dir.clone();
        let shared = shared.clone();
        move |id: SharedString| {
            let Some(w) = weak.upgrade() else { return };
            let Ok(state) = store::load(&dir) else { return };
            let id = id.to_string();
            let Some(sub) = state.subscriptions.iter().find(|s| s.id == id) else {
                return;
            };
            lock(&shared).sheet_id = id;
            w.set_sheet_name(sub.name.clone().into());
            w.set_sheet_ignore(sub.options.ignore_provider_routing);
            w.set_sheet_delete_armed(false);
            w.set_sheet_open(true);
        }
    });

    window.on_sub_save({
        let weak = window.as_weak();
        let dir = data_dir.clone();
        let shared = shared.clone();
        move |name: SharedString, ignore: bool| {
            let Some(w) = weak.upgrade() else { return };
            let id = lock(&shared).sheet_id.clone();
            let name = name.trim().to_string();
            let result = phone::edit_sub(&dir, &id, |sub| {
                sub.name = name;
                sub.options.ignore_provider_routing = ignore;
            });
            show(&w, &shared, result);
        }
    });

    window.on_sub_delete({
        let weak = window.as_weak();
        let dir = data_dir.clone();
        let shared = shared.clone();
        move || {
            let Some(w) = weak.upgrade() else { return };
            let id = lock(&shared).sheet_id.clone();
            let result = phone::remove_subscription(&dir, &id);
            show(&w, &shared, result);
        }
    });

    // Routing tab. Every change saves the state and refreshes the tab.
    window.on_routing_preset_changed({
        let weak = window.as_weak();
        let dir = data_dir.clone();
        let shared = shared.clone();
        move |index: i32| {
            let Some(w) = weak.upgrade() else { return };
            let preset = phone::preset_from_index(index);
            show(
                &w,
                &shared,
                phone::update_state(&dir, |st| st.settings.routing.preset = preset),
            );
        }
    });
    window.on_routing_mirror_changed({
        let weak = window.as_weak();
        let dir = data_dir.clone();
        let shared = shared.clone();
        move |index: i32| {
            let Some(w) = weak.upgrade() else { return };
            let mirror = if index == 1 {
                MirrorMode::Proxy
            } else {
                MirrorMode::Direct
            };
            show(
                &w,
                &shared,
                phone::update_state(&dir, |st| st.settings.routing.anixart_mirror = mirror),
            );
        }
    });
    window.on_routing_strategy_changed({
        let weak = window.as_weak();
        let dir = data_dir.clone();
        let shared = shared.clone();
        move |index: i32| {
            let Some(w) = weak.upgrade() else { return };
            let name = usize::try_from(index)
                .ok()
                .and_then(|i| phone::DOMAIN_STRATEGIES.get(i))
                .map_or("IPIfNonMatch", |s| s);
            show(
                &w,
                &shared,
                phone::update_state(&dir, |st| st.settings.routing.domain_strategy = name.into()),
            );
        }
    });
    window.on_routing_quic_changed({
        let weak = window.as_weak();
        let dir = data_dir.clone();
        let shared = shared.clone();
        move |on: bool| {
            let Some(w) = weak.upgrade() else { return };
            show(
                &w,
                &shared,
                phone::update_state(&dir, |st| st.settings.routing.block_quic = on),
            );
        }
    });
    window.on_rule_add({
        let weak = window.as_weak();
        let dir = data_dir.clone();
        let shared = shared.clone();
        move || {
            let Some(w) = weak.upgrade() else { return };
            show(
                &w,
                &shared,
                phone::update_state(&dir, |st| {
                    st.settings.routing.custom_rules.push(RoutingRule {
                        domains: Vec::new(),
                        ips: Vec::new(),
                        processes: Vec::new(),
                        network: None,
                        ports: None,
                        action: RuleAction::Proxy,
                        enabled: true,
                    });
                }),
            );
        }
    });
    window.on_rule_action_changed({
        let weak = window.as_weak();
        let dir = data_dir.clone();
        let shared = shared.clone();
        move |index: i32, action: i32| {
            let Some(w) = weak.upgrade() else { return };
            let action = match action {
                1 => RuleAction::Direct,
                2 => RuleAction::Block,
                _ => RuleAction::Proxy,
            };
            show(
                &w,
                &shared,
                phone::update_state(&dir, |st| {
                    if let Some(rule) = rule_at(st, index) {
                        rule.action = action;
                    }
                }),
            );
        }
    });
    window.on_rule_enabled_changed({
        let weak = window.as_weak();
        let dir = data_dir.clone();
        let shared = shared.clone();
        move |index: i32, on: bool| {
            let Some(w) = weak.upgrade() else { return };
            show(
                &w,
                &shared,
                phone::update_state(&dir, |st| {
                    if let Some(rule) = rule_at(st, index) {
                        rule.enabled = on;
                    }
                }),
            );
        }
    });
    window.on_rule_delete({
        let weak = window.as_weak();
        let dir = data_dir.clone();
        let shared = shared.clone();
        move |index: i32| {
            let Some(w) = weak.upgrade() else { return };
            show(
                &w,
                &shared,
                phone::update_state(&dir, |st| {
                    if let Ok(i) = usize::try_from(index) {
                        if i < st.settings.routing.custom_rules.len() {
                            st.settings.routing.custom_rules.remove(i);
                        }
                    }
                }),
            );
        }
    });
    // Typing: saved without a refresh, so the text field keeps its cursor.
    window.on_rule_values_changed({
        let weak = window.as_weak();
        let dir = data_dir.clone();
        move |index: i32, kind: i32, text: SharedString| {
            let Some(w) = weak.upgrade() else { return };
            let values = phone::rule_values(&text);
            if let Ok(state) = phone::update_state(&dir, |st| {
                if let Some(rule) = rule_at(st, index) {
                    match kind {
                        1 => rule.ips = values,
                        2 => rule.processes = values,
                        _ => rule.domains = values,
                    }
                }
            }) {
                w.set_routing_error(phone::routing_error(&state).into());
            }
        }
    });
    window.on_provider_enabled_changed({
        let weak = window.as_weak();
        let dir = data_dir.clone();
        let shared = shared.clone();
        move |id: SharedString, on: bool| {
            let Some(w) = weak.upgrade() else { return };
            let result = phone::edit_sub(&dir, &id, |sub| {
                if let Some(routing) = sub.provider.routing.as_mut() {
                    routing.enabled = on;
                }
            });
            show(&w, &shared, result);
        }
    });
    window.on_provider_ignored_changed({
        let weak = window.as_weak();
        let dir = data_dir.clone();
        let shared = shared.clone();
        move |id: SharedString, on: bool| {
            let Some(w) = weak.upgrade() else { return };
            let result = phone::edit_sub(&dir, &id, |sub| {
                sub.options.ignore_provider_routing = on;
            });
            show(&w, &shared, result);
        }
    });

    // Settings tab.
    window.on_core_changed({
        let weak = window.as_weak();
        let dir = data_dir.clone();
        let shared = shared.clone();
        move |index: i32| {
            let Some(w) = weak.upgrade() else { return };
            let name = usize::try_from(index)
                .ok()
                .and_then(|i| phone::CORE_OVERRIDES.get(i))
                .map_or("auto", |c| c);
            show(
                &w,
                &shared,
                phone::update_state(&dir, |st| st.settings.core_override = name.into()),
            );
        }
    });
    window.on_theme_changed({
        let weak = window.as_weak();
        let dir = data_dir.clone();
        let shared = shared.clone();
        move |index: i32| {
            let Some(w) = weak.upgrade() else { return };
            let theme = match index {
                1 => flamix_core::model::Theme::Dark,
                2 => flamix_core::model::Theme::Light,
                _ => flamix_core::model::Theme::System,
            };
            show(
                &w,
                &shared,
                phone::update_state(&dir, |st| st.settings.theme = theme),
            );
        }
    });
    window.on_language_changed({
        let weak = window.as_weak();
        let dir = data_dir.clone();
        let shared = shared.clone();
        move |index: i32| {
            let Some(w) = weak.upgrade() else { return };
            let language = if index == 1 { "en" } else { "ru" };
            show(
                &w,
                &shared,
                phone::update_state(&dir, |st| st.settings.language = language.into()),
            );
        }
    });
    window.on_dns_save({
        let weak = window.as_weak();
        let dir = data_dir.clone();
        let shared = shared.clone();
        move || {
            let Some(w) = weak.upgrade() else { return };
            let remote = w.get_dns_remote().trim().to_string();
            let direct = w.get_dns_direct().trim().to_string();
            let bootstrap = w.get_dns_bootstrap().trim().to_string();
            if let Err(err) = phone::check_dns(&remote, &direct) {
                w.set_status_text(err.into());
                return;
            }
            let result = phone::update_state(&dir, |st| {
                st.settings.dns.remote = remote;
                st.settings.dns.direct = direct;
                st.settings.dns.bootstrap = bootstrap;
            });
            match result {
                Ok(state) => {
                    load_settings_fields(&w, &state);
                    show(&w, &shared, Ok(state));
                    w.set_status_text("DNS сохранён".into());
                }
                Err(err) => w.set_status_text(err.into()),
            }
        }
    });
    window.on_ping_save({
        let weak = window.as_weak();
        let dir = data_dir.clone();
        let shared = shared.clone();
        move || {
            let Some(w) = weak.upgrade() else { return };
            let url = w.get_ping_url().trim().to_string();
            if phone::safe_url(&url)
                .filter(|u| u.starts_with("http"))
                .is_none()
            {
                w.set_status_text("Адрес должен начинаться с http:// или https://".into());
                return;
            }
            let result = phone::update_state(&dir, |st| st.settings.ping_url = url);
            match result {
                Ok(state) => {
                    load_settings_fields(&w, &state);
                    show(&w, &shared, Ok(state));
                    w.set_status_text("Адрес сохранён".into());
                }
                Err(err) => w.set_status_text(err.into()),
            }
        }
    });
    window.on_long_press_start({
        let weak = window.as_weak();
        let dir = data_dir.clone();
        let shared = shared.clone();
        move || {
            let press = {
                let mut ui = lock(&shared);
                ui.press_seq += 1;
                ui.press = ui.press_seq;
                ui.press
            };
            let weak = weak.clone();
            let dir = dir.clone();
            let shared = shared.clone();
            slint::Timer::single_shot(Duration::from_millis(700), move || {
                {
                    let mut ui = lock(&shared);
                    if ui.press != press {
                        return;
                    }
                    ui.press = 0;
                }
                let Some(w) = weak.upgrade() else { return };
                open_device_id_editor(&w, &dir);
            });
        }
    });
    window.on_long_press_end({
        let shared = shared.clone();
        move || {
            lock(&shared).press = 0;
        }
    });
    window.on_hwid_save({
        let weak = window.as_weak();
        let dir = data_dir.clone();
        let shared = shared.clone();
        move || {
            let Some(w) = weak.upgrade() else { return };
            let input = w.get_hwid_input().to_string();
            match phone::hwid_from_input(&input) {
                Ok(value) => {
                    let result = phone::update_state(&dir, |st| st.hwid_override = value);
                    w.set_hwid_open(false);
                    show(&w, &shared, result);
                }
                Err(err) => w.set_hwid_error(err.into()),
            }
        }
    });
    window.on_hwid_reset({
        let weak = window.as_weak();
        let dir = data_dir.clone();
        let shared = shared.clone();
        move || {
            let Some(w) = weak.upgrade() else { return };
            w.set_hwid_open(false);
            show(
                &w,
                &shared,
                phone::update_state(&dir, |st| st.hwid_override = None),
            );
        }
    });
    window.on_hwid_close({
        let weak = window.as_weak();
        move || {
            if let Some(w) = weak.upgrade() {
                w.set_hwid_open(false);
            }
        }
    });

    window.on_open_url(move |url: SharedString| {
        let Some(url) = phone::safe_url(&url) else {
            return;
        };
        if let Err(err) = external::open(&url) {
            applog::log("E", "ui", &format!("open link failed: {err}"));
        }
    });

    window.on_connect_pressed({
        let weak = window.as_weak();
        let dir = data_dir.clone();
        move || {
            let Some(w) = weak.upgrade() else { return };
            if vpn::is_running() {
                applog::log("I", "ui", "disconnect pressed");
                let text = match vpn::disconnect() {
                    Ok(()) => "Отключение…".to_string(),
                    Err(err) => err,
                };
                w.set_status_text(text.into());
                return;
            }
            applog::log("I", "ui", "connect pressed");
            let result = store::load(&dir)
                .map_err(|e| e.to_string())
                .and_then(|state| phone::vpn_config_json(&state))
                .and_then(|json| vpn::connect(&json));
            let text = match result {
                Ok(()) => "Запрошено подключение VPN".to_string(),
                Err(err) => {
                    applog::log("E", "ui", &format!("connect failed: {err}"));
                    err
                }
            };
            w.set_status_text(text.into());
        }
    });

    window.on_add_subscription({
        let weak = window.as_weak();
        let dir = data_dir.clone();
        let shared = shared.clone();
        move || {
            let Some(w) = weak.upgrade() else { return };
            let mut sub = match phone::new_subscription(&w.get_add_url()) {
                Ok(sub) => sub,
                Err(err) => {
                    w.set_add_status(err.into());
                    return;
                }
            };
            sub.name = w.get_add_name().trim().to_string();
            sub.backup_urls = phone::parse_backup_urls(&w.get_add_backups());
            let id = sub.id.clone();
            match phone::add_subscription(&dir, sub) {
                Ok(state) => {
                    refresh(&w, &state, &shared);
                    w.set_add_url("".into());
                    w.set_add_name("".into());
                    w.set_add_backups("".into());
                    w.set_add_status("Подписка добавлена, загрузка…".into());
                    start_refresh(&w, &dir, &shared, id);
                }
                Err(err) => w.set_add_status(err.into()),
            }
        }
    });

    window.on_import_links({
        let weak = window.as_weak();
        let dir = data_dir.clone();
        let shared = shared.clone();
        move || {
            let Some(w) = weak.upgrade() else { return };
            let text = w.get_links_text().to_string();
            if text.trim().is_empty() {
                return;
            }
            if import_and_report(&w, &dir, &shared, &text) {
                w.set_links_text("".into());
            }
        }
    });

    window.on_import_clipboard({
        let weak = window.as_weak();
        let dir = data_dir.clone();
        let shared = shared.clone();
        move || {
            let Some(w) = weak.upgrade() else { return };
            match external::read_clipboard() {
                Ok(text) if !text.trim().is_empty() => {
                    import_and_report(&w, &dir, &shared, &text);
                }
                Ok(_) => w.set_add_status("Буфер обмена пуст".into()),
                Err(err) => w.set_add_status(err.into()),
            }
        }
    });

    // The Kotlin side owns the VPN state; poll it so the button shows the real status.
    let poll = Timer::default();
    poll.start(TimerMode::Repeated, Duration::from_secs(1), {
        let weak = window.as_weak();
        move || {
            let Some(w) = weak.upgrade() else { return };
            let running = vpn::is_running();
            w.set_connected(running);
            w.set_connect_label(
                if running {
                    "Отключить"
                } else {
                    "Подключить"
                }
                .into(),
            );
        }
    });

    window.run()
}

/// Called by `android-activity` from the `NativeActivity` glue.
#[cfg(target_os = "android")]
#[no_mangle]
fn android_main(app: slint::android::AndroidApp) {
    let data_dir = app
        .internal_data_path()
        .unwrap_or_else(|| PathBuf::from("."));
    applog::init(&data_dir);
    applog::log("I", "android_main", "start");
    bridge::init(app.vm_as_ptr(), app.activity_as_ptr());
    external::init(app.vm_as_ptr(), app.activity_as_ptr());
    match slint::android::init(app) {
        Ok(()) => {
            if let Err(e) = run(data_dir) {
                applog::log("E", "android_main", &format!("ui exited with error: {e}"));
            }
        }
        Err(e) => applog::log("E", "android_main", &format!("slint init failed: {e}")),
    }
}

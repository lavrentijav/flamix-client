//! Page "Настройки" (T17B).
//!
//! The page keeps a working copy of [`Settings`], mirrors it into the `SettingsBridge` global and
//! writes it back to the controller with a 500 ms debounce. Pure helpers (validation, enum/string
//! mapping, core-affecting comparison) are unit-tested at the bottom.

use crate::controller::Controller;
use crate::{AppWindow, SettingsBridge};
use chrono::TimeZone;
use flamix_core::model::{
    ConnectionMode, Dns, GeoData, GeoSource, Settings, Theme as SettingsTheme,
};
use slint::{ComponentHandle, Timer, TimerMode};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

/// Debounce before an edit is persisted / applied to the running core.
const SAVE_DEBOUNCE: Duration = Duration::from_millis(500);
/// How long "Сохранено" stays on screen.
const STATUS_TIMEOUT: Duration = Duration::from_secs(2);
/// `AppWindow.page` value of this page (used to poll the geo-updated label while visible).
const PAGE_INDEX: i32 = 3;
/// How often the "Обновлено" label is re-read from the controller state while the page is shown.
const GEO_REFRESH: Duration = Duration::from_secs(2);
/// Static timestamp shown for the `--demo`/screenshot build.
const DEMO_GEO_UPDATED: &str = "16.10.2026 14:30";

thread_local! {
    /// Keeps the repeating geo-label timer alive for the lifetime of the process.
    static GEO_TIMER: RefCell<Option<Timer>> = const { RefCell::new(None) };
}

pub fn init(ui: &AppWindow, ctrl: Option<&Arc<Controller>>) {
    let bridge = ui.global::<SettingsBridge>();
    let initial = match ctrl {
        Some(ctrl) => ctrl.with_state(|state| state.settings.clone()),
        None => demo_settings(),
    };
    write_bridge(&bridge, &initial);
    publish_errors(&bridge, &validate(&initial));
    apply_theme(ui, initial.theme);
    bridge.set_save_status("".into());
    bridge.set_reset_confirm(false);
    let geo_updated = match ctrl {
        Some(ctrl) => ctrl.with_state(|state| geo_updated_text(state.geo_updated)),
        None => DEMO_GEO_UPDATED.to_string(),
    };
    bridge.set_geo_updated_text(geo_updated.into());

    let current = Rc::new(RefCell::new(initial));
    let save_timer = Rc::new(RefCell::new(Timer::default()));
    let status_timer = Rc::new(RefCell::new(Timer::default()));

    // Any edit: refresh the working copy, show inline errors immediately, apply the theme and
    // (re)start the debounce that persists + reconnects.
    bridge.on_changed({
        let ui_weak = ui.as_weak();
        let current = current.clone();
        let save_timer = save_timer.clone();
        let status_timer = status_timer.clone();
        let ctrl = ctrl.cloned();
        move || {
            let Some(ui) = ui_weak.upgrade() else { return };
            let bridge = ui.global::<SettingsBridge>();
            // A stale timer must not blank the status we are about to show.
            status_timer.borrow().stop();
            // The connection panel owns the same setting: mirror the live mode back so the two
            // controls cannot drift apart.
            if let Some(ctrl) = ctrl.as_ref() {
                let tun = ctrl.with_state(|state| state.settings.mode) == ConnectionMode::Tun;
                bridge.set_tun_mode(tun);
            }
            let settings = read_bridge(&bridge, &current.borrow());
            publish_errors(&bridge, &validate(&settings));
            apply_theme(&ui, settings.theme);
            *current.borrow_mut() = settings;

            bridge.set_save_status("Сохранение…".into());
            let ui_weak = ui.as_weak();
            let current = current.clone();
            let status_timer = status_timer.clone();
            let ctrl = ctrl.clone();
            save_timer
                .borrow()
                .start(TimerMode::SingleShot, SAVE_DEBOUNCE, move || {
                    let Some(ui) = ui_weak.upgrade() else { return };
                    let bridge = ui.global::<SettingsBridge>();
                    let settings = current.borrow().clone();
                    if !validate(&settings).is_empty() {
                        bridge.set_save_status("Исправьте ошибки".into());
                        return;
                    }
                    save(&ui, ctrl.as_ref(), &settings);
                    bridge.set_save_status("Сохранено".into());
                    let ui_weak = ui.as_weak();
                    status_timer
                        .borrow()
                        .start(TimerMode::SingleShot, STATUS_TIMEOUT, move || {
                            if let Some(ui) = ui_weak.upgrade() {
                                ui.global::<SettingsBridge>().set_save_status("".into());
                            }
                        });
                });
        }
    });

    // The Proxy/TUN control writes the mode immediately (and reconnects): it is shared with the
    // connection panel, so it must not go through the debounced snapshot save.
    bridge.on_mode_changed({
        let current = current.clone();
        let ctrl = ctrl.cloned();
        move |tun| {
            let mode = if tun {
                ConnectionMode::Tun
            } else {
                ConnectionMode::Proxy
            };
            current.borrow_mut().mode = mode;
            let Some(ctrl) = ctrl.as_ref() else { return };
            ctrl.mutate(move |state| state.settings.mode = mode);
            ctrl.reconnect_if_connected();
        }
    });

    // Reset to defaults (already confirmed on the page).
    bridge.on_reset_confirmed({
        let ui_weak = ui.as_weak();
        let current = current.clone();
        let status_timer = status_timer.clone();
        let ctrl = ctrl.cloned();
        move || {
            let Some(ui) = ui_weak.upgrade() else { return };
            let bridge = ui.global::<SettingsBridge>();
            // Cancel any pending "Сохранено" timer that could blank the new status.
            status_timer.borrow().stop();
            let defaults = Settings::default();
            write_bridge(&bridge, &defaults);
            publish_errors(&bridge, &validate(&defaults));
            *current.borrow_mut() = defaults.clone();
            bridge.set_reset_confirm(false);
            // Preserve mode/routing (owned elsewhere) while resetting the rest.
            save(&ui, ctrl.as_ref(), &defaults);
            bridge.set_save_status("Сброшено".into());
            let ui_weak = ui.as_weak();
            status_timer
                .borrow()
                .start(TimerMode::SingleShot, STATUS_TIMEOUT, move || {
                    if let Some(ui) = ui_weak.upgrade() {
                        ui.global::<SettingsBridge>().set_save_status("".into());
                    }
                });
        }
    });

    bridge.on_reset_cancelled({
        let ui_weak = ui.as_weak();
        move || {
            if let Some(ui) = ui_weak.upgrade() {
                ui.global::<SettingsBridge>().set_reset_confirm(false);
            }
        }
    });

    // "Обновить геоданные": the controller downloads asynchronously and bumps `geo_updated`
    // when it finishes, so the label is refreshed by the polling timer below (and here in
    // demo mode there is no controller, so we only show a demo status line).
    bridge.on_update_geo({
        let ui_weak = ui.as_weak();
        let ctrl = ctrl.cloned();
        move || {
            let Some(ui) = ui_weak.upgrade() else { return };
            match ctrl.as_ref() {
                Some(ctrl) => ctrl.update_geo(),
                None => ui
                    .global::<SettingsBridge>()
                    .set_save_status("Демо: геоданные обновлены".into()),
            }
        }
    });

    // Re-read "Обновлено" from the controller: `update_geo` and a connect both finish on the
    // runtime thread, so the label would otherwise stay stale. The timer runs process-wide and
    // early-returns unless this page (index 3) is visible (same pattern as the stats page).
    let ui_weak = ui.as_weak();
    let ctrl = ctrl.cloned();
    let timer = Timer::default();
    timer.start(TimerMode::Repeated, GEO_REFRESH, move || {
        let Some(ui) = ui_weak.upgrade() else { return };
        if ui.get_page() != PAGE_INDEX {
            return;
        }
        if let Some(ctrl) = ctrl.as_ref() {
            let text = ctrl.with_state(|state| geo_updated_text(state.geo_updated));
            ui.global::<SettingsBridge>()
                .set_geo_updated_text(text.into());
        }
    });
    GEO_TIMER.with(|slot| *slot.borrow_mut() = Some(timer));
}

/// Persist the fields this page owns through the controller and reconnect when the core must be
/// restarted. The write starts from the live state so fields owned elsewhere (`mode`, `routing`)
/// are never reverted to a stale snapshot.
fn save(ui: &AppWindow, ctrl: Option<&Arc<Controller>>, settings: &Settings) {
    apply_theme(ui, settings.theme);
    let Some(ctrl) = ctrl else { return };

    let live_before = ctrl.with_state(|state| state.settings.clone());
    let effective = merge_owned(&live_before, settings);
    let reconnect = core_affecting_changed(&live_before, &effective);
    let launch_changed = live_before.launch_at_login != effective.launch_at_login;
    let launch_at_login = effective.launch_at_login;
    // Changing the source (or a custom URL) invalidates the databases downloaded for the
    // previous source, so reset `geo_updated` to mark the data stale (the label then reads
    // "никогда"). Note: `flamix_core::geo` decides freshness from the file mtime, so this reset
    // is a UI/marker only; the re-download itself is forced by the "Обновить геоданные"
    // button (`max_age = 0`) or by the connect-time refresh once the files age out.
    let geo_changed = live_before.geo != effective.geo;

    let src = settings.clone();
    ctrl.mutate(move |state| {
        apply_owned_in_place(&mut state.settings, &src);
        if geo_changed {
            state.geo_updated = 0;
        }
    });
    if reconnect {
        ctrl.reconnect_if_connected();
    }
    if launch_changed {
        let handle = ctrl.runtime().spawn_blocking(move || {
            if let Err(err) = flamix_sys::autostart::set_enabled(launch_at_login) {
                tracing::warn!(error = %err, "failed to update autostart");
            }
        });
        drop(handle);
    }
}

/// Copy the fields owned by this page from `src` into `live`, leaving `mode` (connection panel)
/// and `routing` (routing page) untouched.
fn apply_owned_in_place(live: &mut Settings, src: &Settings) {
    live.socks_port = src.socks_port;
    live.http_port = src.http_port;
    live.allow_lan = src.allow_lan;
    live.set_system_proxy = src.set_system_proxy;
    live.proxy_bypass = src.proxy_bypass.clone();
    live.udp_enabled = src.udp_enabled;
    live.sniffing = src.sniffing;
    live.dest_override = src.dest_override.clone();
    live.route_only = src.route_only;
    live.local_auth_user = src.local_auth_user.clone();
    live.local_auth_pass = src.local_auth_pass.clone();
    live.dns = src.dns.clone();
    live.geo = src.geo.clone();
    live.tun_mtu = src.tun_mtu;
    live.tun_stack = src.tun_stack.clone();
    live.tun_ipv6 = src.tun_ipv6;
    live.tun_auto_route = src.tun_auto_route;
    live.tun_strict_route = src.tun_strict_route;
    live.tun_icmp = src.tun_icmp.clone();
    live.tun_route_exclude = src.tun_route_exclude.clone();
    live.mux = src.mux;
    live.mux_concurrency = src.mux_concurrency;
    live.core_override = src.core_override.clone();
    live.default_fingerprint = src.default_fingerprint.clone();
    live.ping_url = src.ping_url.clone();
    live.ping_method = src.ping_method.clone();
    live.launch_at_login = src.launch_at_login;
    live.connect_on_launch = src.connect_on_launch;
    live.minimize_to_tray = src.minimize_to_tray;
    live.auto_update_subscriptions = src.auto_update_subscriptions;
    live.theme = src.theme;
    live.language = src.language.clone();
    live.log_level = src.log_level.clone();
}

/// `live` with this page's fields taken from `src` (used for the change comparison).
fn merge_owned(live: &Settings, src: &Settings) -> Settings {
    let mut out = live.clone();
    apply_owned_in_place(&mut out, src);
    out
}

/// Mirror the theme into the UI immediately (System follows the app default = dark).
fn apply_theme(ui: &AppWindow, theme: SettingsTheme) {
    super::apply_theme(ui, theme_is_dark(theme));
}

fn theme_is_dark(theme: SettingsTheme) -> bool {
    match theme {
        SettingsTheme::Dark => true,
        SettingsTheme::Light => false,
        SettingsTheme::System => true,
    }
}

fn demo_settings() -> Settings {
    Settings {
        mux: true,
        allow_lan: true,
        ..Settings::default()
    }
}

fn write_bridge(bridge: &SettingsBridge, s: &Settings) {
    bridge.set_tun_mode(s.mode == ConnectionMode::Tun);
    bridge.set_socks_port(i32::from(s.socks_port));
    bridge.set_http_port(i32::from(s.http_port));
    bridge.set_allow_lan(s.allow_lan);
    bridge.set_set_system_proxy(s.set_system_proxy);
    bridge.set_bypass(bypass_to_text(&s.proxy_bypass).into());
    bridge.set_udp_enabled(s.udp_enabled);
    bridge.set_sniffing(s.sniffing);
    let (dest_http, dest_tls, dest_quic, dest_fakedns) = dest_override_flags(&s.dest_override);
    bridge.set_dest_http(dest_http);
    bridge.set_dest_tls(dest_tls);
    bridge.set_dest_quic(dest_quic);
    bridge.set_dest_fakedns(dest_fakedns);
    bridge.set_route_only(s.route_only);
    bridge.set_local_auth_user(s.local_auth_user.clone().into());
    bridge.set_local_auth_pass(s.local_auth_pass.clone().into());
    bridge.set_mux(s.mux);
    bridge.set_mux_concurrency(s.mux_concurrency as i32);
    bridge.set_core_override(s.core_override.clone().into());
    bridge.set_fingerprint(s.default_fingerprint.clone().into());
    bridge.set_tun_stack(s.tun_stack.clone().into());
    bridge.set_tun_mtu(s.tun_mtu as i32);
    bridge.set_tun_ipv6(s.tun_ipv6);
    bridge.set_tun_auto_route(s.tun_auto_route);
    bridge.set_tun_strict_route(s.tun_strict_route);
    bridge.set_tun_icmp(s.tun_icmp.clone().into());
    bridge.set_route_exclude(list_to_text(&s.tun_route_exclude).into());
    bridge.set_dns_remote(s.dns.remote.clone().into());
    bridge.set_dns_direct(s.dns.direct.clone().into());
    bridge.set_dns_fake_ip(s.dns.fake_ip);
    bridge.set_dns_fake_ip_range(s.dns.fake_ip_range.clone().into());
    bridge.set_dns_bootstrap(s.dns.bootstrap.clone().into());
    bridge.set_dns_block_svcb(s.dns.block_svcb);
    bridge.set_dns_parallel(s.dns.parallel);
    bridge.set_dns_serve_stale(s.dns.serve_stale);
    bridge.set_dns_hosts(hosts_to_text(&s.dns.hosts).into());
    bridge.set_geo_source_index(source_index(s.geo.source));
    bridge.set_geo_geosite_url(s.geo.geosite_url.clone().into());
    bridge.set_geo_geoip_url(s.geo.geoip_url.clone().into());
    bridge.set_ping_method(s.ping_method.clone().into());
    bridge.set_ping_url(s.ping_url.clone().into());
    bridge.set_launch_at_login(s.launch_at_login);
    bridge.set_connect_on_launch(s.connect_on_launch);
    bridge.set_minimize_to_tray(s.minimize_to_tray);
    bridge.set_auto_update(s.auto_update_subscriptions);
    bridge.set_theme_index(theme_index(s.theme));
    bridge.set_language_index(language_index(&s.language));
    bridge.set_log_level(s.log_level.clone().into());
}

fn read_bridge(bridge: &SettingsBridge, base: &Settings) -> Settings {
    Settings {
        mode: if bridge.get_tun_mode() {
            ConnectionMode::Tun
        } else {
            ConnectionMode::Proxy
        },
        socks_port: bridge.get_socks_port().clamp(1, 65_535) as u16,
        http_port: bridge.get_http_port().clamp(1, 65_535) as u16,
        allow_lan: bridge.get_allow_lan(),
        set_system_proxy: bridge.get_set_system_proxy(),
        proxy_bypass: bypass_from_text(&bridge.get_bypass()),
        udp_enabled: bridge.get_udp_enabled(),
        sniffing: bridge.get_sniffing(),
        dest_override: dest_override_from_flags(
            bridge.get_dest_http(),
            bridge.get_dest_tls(),
            bridge.get_dest_quic(),
            bridge.get_dest_fakedns(),
        ),
        route_only: bridge.get_route_only(),
        local_auth_user: bridge.get_local_auth_user().to_string(),
        local_auth_pass: bridge.get_local_auth_pass().to_string(),
        routing: base.routing.clone(),
        failover: base.failover.clone(),
        geo: {
            let mut geo = GeoData {
                source: source_from_index(bridge.get_geo_source_index()),
                geosite_url: bridge.get_geo_geosite_url().to_string(),
                geoip_url: bridge.get_geo_geoip_url().to_string(),
            };
            trim_geo_urls(&mut geo);
            geo
        },
        dns: Dns {
            remote: bridge.get_dns_remote().to_string(),
            direct: bridge.get_dns_direct().to_string(),
            fake_ip: bridge.get_dns_fake_ip(),
            fake_ip_range: bridge.get_dns_fake_ip_range().to_string(),
            bootstrap: bridge.get_dns_bootstrap().to_string(),
            block_svcb: bridge.get_dns_block_svcb(),
            parallel: bridge.get_dns_parallel(),
            serve_stale: bridge.get_dns_serve_stale(),
            hosts: hosts_from_text(&bridge.get_dns_hosts()),
        },
        tun_mtu: bridge.get_tun_mtu().max(0) as u32,
        tun_stack: bridge.get_tun_stack().to_string(),
        tun_ipv6: bridge.get_tun_ipv6(),
        tun_auto_route: bridge.get_tun_auto_route(),
        tun_strict_route: bridge.get_tun_strict_route(),
        tun_icmp: bridge.get_tun_icmp().to_string(),
        tun_route_exclude: list_from_text(&bridge.get_route_exclude()),
        // TProxy / AP tether fields are not yet exposed in the Slint settings bridge
        // (M3 of transparent-proxy); carry through whatever is in the base snapshot so
        // state round-trips untouched.
        tproxy_port: base.tproxy_port,
        tproxy_mark: base.tproxy_mark,
        tproxy_table: base.tproxy_table,
        tproxy_route_exclude: base.tproxy_route_exclude.clone(),
        ap_tether_enabled: base.ap_tether_enabled,
        ap_interfaces: base.ap_interfaces.clone(),
        mux: bridge.get_mux(),
        mux_concurrency: bridge.get_mux_concurrency().max(1) as u32,
        core_override: {
            let value = bridge.get_core_override().to_string();
            if value.is_empty() {
                "auto".into()
            } else {
                value
            }
        },
        default_fingerprint: bridge.get_fingerprint().to_string(),
        ping_url: bridge.get_ping_url().to_string(),
        ping_method: bridge.get_ping_method().to_string(),
        launch_at_login: bridge.get_launch_at_login(),
        connect_on_launch: bridge.get_connect_on_launch(),
        minimize_to_tray: bridge.get_minimize_to_tray(),
        auto_update_subscriptions: bridge.get_auto_update(),
        theme: theme_from_index(bridge.get_theme_index()),
        language: language_from_index(bridge.get_language_index()).to_string(),
        log_level: bridge.get_log_level().to_string(),
    }
}

/// Inline validation errors; empty string means "no error".
#[derive(Debug, Default, PartialEq, Eq)]
struct Errors {
    ports: String,
    dns: String,
    ping: String,
    route_exclude: String,
    geo: String,
}

impl Errors {
    fn is_empty(&self) -> bool {
        self.ports.is_empty()
            && self.dns.is_empty()
            && self.ping.is_empty()
            && self.route_exclude.is_empty()
            && self.geo.is_empty()
    }
}

fn validate(s: &Settings) -> Errors {
    let mut errors = Errors::default();
    if s.socks_port == s.http_port {
        errors.ports = "Порты SOCKS и HTTP должны различаться".into();
    }

    let mut bad_dns = Vec::new();
    if !is_valid_dns(&s.dns.remote) {
        bad_dns.push("удалённый");
    }
    if !is_valid_dns(&s.dns.direct) {
        bad_dns.push("прямой");
    }
    if s.dns.fake_ip && !is_valid_cidr(&s.dns.fake_ip_range) {
        bad_dns.push("диапазон fake-ip");
    }
    if !s.dns.bootstrap.trim().is_empty() && !is_valid_dns(&s.dns.bootstrap) {
        bad_dns.push("bootstrap");
    }
    if !bad_dns.is_empty() {
        errors.dns = format!("Некорректный {} DNS", bad_dns.join(", "));
    }

    let bad_excludes: Vec<&str> = s
        .tun_route_exclude
        .iter()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty() && !is_valid_cidr(value))
        .collect();
    if !bad_excludes.is_empty() {
        errors.route_exclude = format!("Некорректный CIDR: {}", bad_excludes.join(", "));
    }

    if s.ping_method == "url" && !is_valid_url(&s.ping_url) {
        errors.ping = "Укажите корректный URL (http:// или https://)".into();
    }

    // Custom geo URLs only matter for `GeoSource::Custom` and may stay empty (= misconfigured,
    // but the controller reports "не настроены" instead of us blocking the save).
    if s.geo.source == GeoSource::Custom {
        let bad: Vec<&str> = [("geosite", &s.geo.geosite_url), ("geoip", &s.geo.geoip_url)]
            .into_iter()
            .filter(|(_, url)| !url.trim().is_empty() && !is_valid_url(url))
            .map(|(name, _)| name)
            .collect();
        if !bad.is_empty() {
            errors.geo = format!(
                "Некорректный URL геоданных (http:// или https://): {}",
                bad.join(", ")
            );
        }
    }
    errors
}

fn publish_errors(bridge: &SettingsBridge, errors: &Errors) {
    bridge.set_ports_error(errors.ports.clone().into());
    bridge.set_dns_error(errors.dns.clone().into());
    bridge.set_ping_error(errors.ping.clone().into());
    bridge.set_route_exclude_error(errors.route_exclude.clone().into());
    bridge.set_geo_urls_error(errors.geo.clone().into());
}

/// True when a change must be applied by restarting the running core.
fn core_affecting_changed(a: &Settings, b: &Settings) -> bool {
    a.mode != b.mode
        || a.socks_port != b.socks_port
        || a.http_port != b.http_port
        || a.allow_lan != b.allow_lan
        || a.set_system_proxy != b.set_system_proxy
        || a.proxy_bypass != b.proxy_bypass
        || a.udp_enabled != b.udp_enabled
        || a.sniffing != b.sniffing
        || a.dest_override != b.dest_override
        || a.route_only != b.route_only
        || a.local_auth_user != b.local_auth_user
        || a.local_auth_pass != b.local_auth_pass
        || a.dns != b.dns
        || a.geo != b.geo
        || a.tun_mtu != b.tun_mtu
        || a.tun_stack != b.tun_stack
        || a.tun_ipv6 != b.tun_ipv6
        || a.tun_auto_route != b.tun_auto_route
        || a.tun_strict_route != b.tun_strict_route
        || a.tun_icmp != b.tun_icmp
        || a.tun_route_exclude != b.tun_route_exclude
        || a.mux != b.mux
        || a.mux_concurrency != b.mux_concurrency
        || a.core_override != b.core_override
        || a.default_fingerprint != b.default_fingerprint
        // Core-level failover is owned by the routing page, but a change to it still needs a
        // reconnect when it lands while this page is saving (documented gap).
        || a.failover != b.failover
}

fn is_valid_url(value: &str) -> bool {
    let value = value.trim();
    let rest = value
        .strip_prefix("https://")
        .or_else(|| value.strip_prefix("http://"));
    match rest {
        Some(host) => {
            let host = host.split(['/', '?', '#']).next().unwrap_or("");
            !host.is_empty() && !host.contains(' ')
        }
        None => false,
    }
}

fn is_valid_dns(value: &str) -> bool {
    let value = value.trim();
    if value.is_empty() {
        return false;
    }
    if value == "localhost" {
        return true;
    }
    if let Some((scheme, rest)) = value.split_once("://") {
        return matches!(
            scheme,
            "http" | "https" | "tcp" | "udp" | "quic" | "tls" | "h3"
        ) && !rest.trim_matches('/').is_empty();
    }
    let host = value.split(':').next().unwrap_or("");
    host.parse::<std::net::IpAddr>().is_ok()
}

/// One item per line (proxy bypass, route-exclude CIDRs, ...).
fn list_to_text(list: &[String]) -> String {
    list.join("\n")
}

fn list_from_text(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect()
}

fn bypass_to_text(list: &[String]) -> String {
    list_to_text(list)
}

fn bypass_from_text(text: &str) -> Vec<String> {
    list_from_text(text)
}

/// `dest_override` <-> the four checkboxes (http/tls/quic/fakedns); order is canonical.
fn dest_override_flags(list: &[String]) -> (bool, bool, bool, bool) {
    let has = |name: &str| list.iter().any(|value| value.eq_ignore_ascii_case(name));
    (has("http"), has("tls"), has("quic"), has("fakedns"))
}

fn dest_override_from_flags(http: bool, tls: bool, quic: bool, fakedns: bool) -> Vec<String> {
    let mut list = Vec::new();
    if http {
        list.push("http".to_string());
    }
    if tls {
        list.push("tls".to_string());
    }
    if quic {
        list.push("quic".to_string());
    }
    if fakedns {
        list.push("fakedns".to_string());
    }
    list
}

/// `dns.hosts` <-> `domain ip` lines (one mapping per line, `domain` first).
fn hosts_to_text(hosts: &BTreeMap<String, String>) -> String {
    hosts
        .iter()
        .map(|(domain, address)| format!("{domain} {address}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn hosts_from_text(text: &str) -> BTreeMap<String, String> {
    let mut hosts = BTreeMap::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let mut parts = line.split_whitespace();
        if let (Some(domain), Some(address)) = (parts.next(), parts.next()) {
            hosts.insert(domain.to_string(), address.to_string());
        }
    }
    hosts
}

/// True for an IP or an `addr/prefix` CIDR (the tun route-exclude format).
fn is_valid_cidr(value: &str) -> bool {
    let value = value.trim();
    match value.split_once('/') {
        Some((addr, prefix)) => {
            let Ok(prefix) = prefix.trim().parse::<u8>() else {
                return false;
            };
            match addr.trim().parse::<std::net::IpAddr>() {
                Ok(std::net::IpAddr::V4(_)) => prefix <= 32,
                Ok(std::net::IpAddr::V6(_)) => prefix <= 128,
                Err(_) => false,
            }
        }
        None => value.parse::<std::net::IpAddr>().is_ok(),
    }
}

/// `geo.source` <-> the "Источник" ComboBox index; order matches the widget model.
fn source_index(source: GeoSource) -> i32 {
    match source {
        GeoSource::FreedomForRussia => 0,
        GeoSource::Loyalsoldier => 1,
        GeoSource::Custom => 2,
    }
}

fn source_from_index(index: i32) -> GeoSource {
    match index {
        1 => GeoSource::Loyalsoldier,
        2 => GeoSource::Custom,
        _ => GeoSource::FreedomForRussia,
    }
}

/// Trim the custom geo URLs so a whitespace-only value is stored as empty (a spaces-only URL
/// would otherwise look non-empty to `flamix_core::geo::urls` and make the download fail).
fn trim_geo_urls(geo: &mut GeoData) {
    geo.geosite_url = geo.geosite_url.trim().to_string();
    geo.geoip_url = geo.geoip_url.trim().to_string();
}

/// Human label for `AppState.geo_updated` (unix seconds): "никогда" at 0, else local date/time.
fn geo_updated_text(updated: i64) -> String {
    if updated <= 0 {
        return "никогда".into();
    }
    match chrono::Local.timestamp_opt(updated, 0).single() {
        Some(dt) => dt.format("%d.%m.%Y %H:%M").to_string(),
        None => "никогда".into(),
    }
}

fn theme_index(theme: SettingsTheme) -> i32 {
    match theme {
        SettingsTheme::System => 0,
        SettingsTheme::Dark => 1,
        SettingsTheme::Light => 2,
    }
}

fn theme_from_index(index: i32) -> SettingsTheme {
    match index {
        1 => SettingsTheme::Dark,
        2 => SettingsTheme::Light,
        _ => SettingsTheme::System,
    }
}

fn language_index(language: &str) -> i32 {
    if language.eq_ignore_ascii_case("en") {
        1
    } else {
        0
    }
}

fn language_from_index(index: i32) -> &'static str {
    if index == 1 {
        "en"
    } else {
        "ru"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flamix_core::model::{RoutingRule, RuleAction};

    fn settings() -> Settings {
        Settings::default()
    }

    #[test]
    fn default_settings_are_valid() {
        assert!(validate(&settings()).is_empty());
    }

    #[test]
    fn equal_ports_are_reported() {
        let s = Settings {
            socks_port: 1080,
            http_port: 1080,
            ..settings()
        };
        assert_eq!(validate(&s).ports, "Порты SOCKS и HTTP должны различаться");
    }

    #[test]
    fn dns_validation_accepts_urls_and_ips() {
        assert!(is_valid_dns("https://1.1.1.1/dns-query"));
        assert!(is_valid_dns("tcp://8.8.8.8"));
        assert!(is_valid_dns("localhost"));
        assert!(is_valid_dns("77.88.8.8"));
        assert!(!is_valid_dns(""));
        assert!(!is_valid_dns("not a dns"));

        let s = Settings {
            dns: Dns {
                remote: "nope".into(),
                direct: "https://77.88.8.8/dns-query".into(),
                fake_ip: false,
                ..Dns::default()
            },
            ..settings()
        };
        assert_eq!(validate(&s).dns, "Некорректный удалённый DNS");
    }

    #[test]
    fn ping_url_required_for_url_method() {
        let s = Settings {
            ping_method: "url".into(),
            ping_url: "example.com".into(),
            ..settings()
        };
        assert!(!validate(&s).ping.is_empty());

        let tcp = Settings {
            ping_method: "tcp".into(),
            ping_url: String::new(),
            ..settings()
        };
        assert!(validate(&tcp).ping.is_empty());
    }

    #[test]
    fn bypass_text_roundtrip_skips_blank_lines() {
        let list = vec!["localhost".to_string(), "10.*".to_string()];
        let text = bypass_to_text(&list);
        assert_eq!(text, "localhost\n10.*");
        let back = bypass_from_text("localhost\n\n  10.*  \n");
        assert_eq!(back, list);
    }

    #[test]
    fn theme_and_language_mapping() {
        for (index, theme) in [
            (0, SettingsTheme::System),
            (1, SettingsTheme::Dark),
            (2, SettingsTheme::Light),
        ] {
            assert_eq!(theme_index(theme), index);
            assert_eq!(theme_from_index(index), theme);
        }
        assert_eq!(theme_from_index(42), SettingsTheme::System);

        assert_eq!(language_index("ru"), 0);
        assert_eq!(language_index("en"), 1);
        assert_eq!(language_from_index(1), "en");
        assert_eq!(language_from_index(0), "ru");
    }

    #[test]
    fn core_affecting_detects_core_fields_only() {
        let base = settings();
        assert!(!core_affecting_changed(&base, &base));

        let port = Settings {
            socks_port: 12345,
            ..base.clone()
        };
        assert!(core_affecting_changed(&base, &port));

        let mux = Settings {
            mux: !base.mux,
            ..base.clone()
        };
        assert!(core_affecting_changed(&base, &mux));

        let system_proxy = Settings {
            set_system_proxy: !base.set_system_proxy,
            ..base.clone()
        };
        assert!(core_affecting_changed(&base, &system_proxy));

        let mut bypass = base.clone();
        bypass.proxy_bypass.push("example.com".into());
        assert!(core_affecting_changed(&base, &bypass));

        // UI-only fields must not trigger a reconnect.
        let ui_only = Settings {
            minimize_to_tray: !base.minimize_to_tray,
            language: "en".into(),
            theme: SettingsTheme::Light,
            ..base.clone()
        };
        assert!(!core_affecting_changed(&base, &ui_only));
    }

    #[test]
    fn theme_is_dark_for_all_variants() {
        assert!(theme_is_dark(SettingsTheme::Dark));
        assert!(!theme_is_dark(SettingsTheme::Light));
        // System follows the app default (dark).
        assert!(theme_is_dark(SettingsTheme::System));
    }

    #[test]
    fn merge_owned_preserves_mode_and_routing() {
        let mut live = settings();
        live.mode = ConnectionMode::Tun;
        live.routing.custom_rules.push(RoutingRule {
            domains: vec!["geosite:category-ru".into()],
            ips: Vec::new(),
            processes: Vec::new(),
            network: None,
            ports: None,
            action: RuleAction::Direct,
            enabled: true,
        });

        let mut edited = live.clone();
        edited.mode = ConnectionMode::Proxy; // stale snapshot from this page
        edited.routing.custom_rules.clear(); // another page owns routing
        edited.mux = !live.mux;

        let merged = merge_owned(&live, &edited);
        assert_eq!(merged.mode, ConnectionMode::Tun);
        assert_eq!(merged.routing.custom_rules.len(), 1);
        assert_eq!(merged.mux, edited.mux);

        // Applying in place from the raw snapshot keeps the live mode/routing too.
        let mut target = live.clone();
        apply_owned_in_place(&mut target, &edited);
        assert_eq!(target.mode, ConnectionMode::Tun);
        assert_eq!(target.routing.custom_rules.len(), 1);
        assert_eq!(target.mux, edited.mux);
    }

    #[test]
    fn list_text_roundtrip_for_route_exclude() {
        let list = vec!["203.0.113.0/24".to_string(), "2001:db8::/32".to_string()];
        assert_eq!(list_to_text(&list), "203.0.113.0/24\n2001:db8::/32");
        assert_eq!(
            list_from_text("203.0.113.0/24\n\n  2001:db8::/32  \n"),
            list
        );
    }

    #[test]
    fn dest_override_flags_roundtrip() {
        let flags = dest_override_flags(&["http".into(), "quic".into(), "FAKEDNS".into()]);
        assert_eq!(flags, (true, false, true, true));
        assert_eq!(
            dest_override_from_flags(true, false, true, true),
            vec!["http", "quic", "fakedns"]
        );
        assert_eq!(
            dest_override_from_flags(false, false, false, false),
            Vec::<String>::new()
        );
    }

    #[test]
    fn hosts_text_roundtrip() {
        let hosts = BTreeMap::from([
            ("a.example".to_string(), "10.0.0.1".to_string()),
            ("b.example".to_string(), "10.0.0.2".to_string()),
        ]);
        // BTreeMap iterates in key order, so the text is deterministic.
        assert_eq!(
            hosts_to_text(&hosts),
            "a.example 10.0.0.1\nb.example 10.0.0.2"
        );
        assert_eq!(
            hosts_from_text("a.example 10.0.0.1\n\n  b.example   10.0.0.2  \n"),
            hosts
        );
        // Lines without exactly two tokens are ignored.
        assert!(hosts_from_text("broken\nonlydomain ").is_empty());
    }

    #[test]
    fn cidr_validation() {
        assert!(is_valid_cidr("10.0.0.0/8"));
        assert!(is_valid_cidr("203.0.113.5"));
        assert!(is_valid_cidr("2001:db8::/32"));
        assert!(!is_valid_cidr("10.0.0.0/33"));
        assert!(!is_valid_cidr("2001:db8::/129"));
        assert!(!is_valid_cidr("nonsense"));
        assert!(!is_valid_cidr("10.0.0.0/"));
    }

    #[test]
    fn validate_reports_bad_route_exclude_and_fake_ip() {
        let s = Settings {
            tun_route_exclude: vec!["10.0.0.0/8".into(), "nope".into()],
            dns: Dns {
                fake_ip: true,
                fake_ip_range: "not-a-cidr".into(),
                ..Dns::default()
            },
            ..settings()
        };
        let errors = validate(&s);
        assert!(errors.route_exclude.contains("nope"));
        assert!(errors.dns.contains("fake-ip"));
    }

    #[test]
    fn core_affecting_detects_new_core_fields() {
        let base = settings();
        for changed in [
            Settings {
                udp_enabled: !base.udp_enabled,
                ..base.clone()
            },
            Settings {
                sniffing: !base.sniffing,
                ..base.clone()
            },
            Settings {
                route_only: !base.route_only,
                ..base.clone()
            },
            Settings {
                tun_strict_route: !base.tun_strict_route,
                ..base.clone()
            },
            Settings {
                tun_icmp: "drop".into(),
                ..base.clone()
            },
            Settings {
                mux_concurrency: 32,
                ..base.clone()
            },
            Settings {
                core_override: "xray".into(),
                ..base.clone()
            },
            Settings {
                dns: Dns {
                    serve_stale: !base.dns.serve_stale,
                    ..base.dns.clone()
                },
                ..base.clone()
            },
        ] {
            assert!(core_affecting_changed(&base, &changed));
        }

        let mut dest = base.clone();
        dest.dest_override.push("quic".into());
        assert!(core_affecting_changed(&base, &dest));

        let mut failover = base.clone();
        failover.failover.enabled = true;
        assert!(core_affecting_changed(&base, &failover));
    }

    #[test]
    fn geo_source_index_mapping_roundtrips() {
        for (index, source) in [
            (0, GeoSource::FreedomForRussia),
            (1, GeoSource::Loyalsoldier),
            (2, GeoSource::Custom),
        ] {
            assert_eq!(source_index(source), index);
            assert_eq!(source_from_index(index), source);
        }
        // Out-of-range indices fall back to the default source.
        assert_eq!(source_from_index(7), GeoSource::FreedomForRussia);
        assert_eq!(source_from_index(-1), GeoSource::FreedomForRussia);
    }

    #[test]
    fn geo_updated_text_handles_never_and_dates() {
        assert_eq!(geo_updated_text(0), "никогда");
        assert_eq!(geo_updated_text(-5), "никогда");
        // A positive timestamp formats as a non-empty local date/time.
        let text = geo_updated_text(1_700_000_000);
        assert!(text.contains('.') && text.contains(':'), "got {text:?}");
    }

    #[test]
    fn custom_geo_urls_are_validated() {
        let mut s = settings();
        s.geo.source = GeoSource::Custom;
        // Empty custom URLs are allowed (the controller reports "не настроены").
        assert!(validate(&s).geo.is_empty());

        s.geo.geosite_url = "https://example.com/geosite.dat".into();
        s.geo.geoip_url = "http://example.com/geoip.dat".into();
        assert!(validate(&s).geo.is_empty());

        s.geo.geosite_url = "example.com/geosite.dat".into();
        assert!(!validate(&s).geo.is_empty());

        // The same bad URL is ignored while the source is not Custom.
        let mut preset = s;
        preset.geo.source = GeoSource::Loyalsoldier;
        assert!(validate(&preset).geo.is_empty());
    }

    #[test]
    fn whitespace_only_geo_urls_validate_and_normalize_to_empty() {
        let mut s = settings();
        s.geo.source = GeoSource::Custom;
        s.geo.geosite_url = "   ".into();
        s.geo.geoip_url = "\t \n".into();
        // A spaces-only URL is treated as empty: no validation error...
        assert!(validate(&s).geo.is_empty());

        // ...and normalization (read_bridge) trims it to a real empty string, so `geo::urls`
        // will not treat it as a URL to download.
        trim_geo_urls(&mut s.geo);
        assert_eq!(s.geo.geosite_url, "");
        assert_eq!(s.geo.geoip_url, "");

        // Surrounding whitespace on a valid URL is also removed.
        s.geo.geosite_url = "  https://example.com/geosite.dat  ".into();
        trim_geo_urls(&mut s.geo);
        assert_eq!(s.geo.geosite_url, "https://example.com/geosite.dat");
    }

    #[test]
    fn geo_change_is_core_affecting_and_merged() {
        let base = settings();
        let custom = Settings {
            geo: GeoData {
                source: GeoSource::Custom,
                geosite_url: "https://example.com/geosite.dat".into(),
                geoip_url: String::new(),
            },
            ..base.clone()
        };
        assert!(core_affecting_changed(&base, &custom));

        // merge_owned/apply_owned_in_place carry `geo` like the other networking fields.
        let mut live = base.clone();
        live.geo.source = GeoSource::Loyalsoldier;
        let merged = merge_owned(&live, &custom);
        assert_eq!(merged.geo, custom.geo);

        let mut target = base.clone();
        apply_owned_in_place(&mut target, &custom);
        assert_eq!(target.geo, custom.geo);
    }

    #[test]
    fn custom_geosite_only_change_is_core_affecting_and_merged() {
        // Source stays Custom: changing only `geosite_url` must still be a core-affecting edit
        // and must survive merge_owned/apply_owned_in_place.
        let base = Settings {
            geo: GeoData {
                source: GeoSource::Custom,
                geosite_url: "https://a.example/geosite.dat".into(),
                geoip_url: "https://a.example/geoip.dat".into(),
            },
            ..settings()
        };
        let edited = Settings {
            geo: GeoData {
                source: GeoSource::Custom,
                geosite_url: "https://b.example/geosite.dat".into(),
                geoip_url: "https://a.example/geoip.dat".into(),
            },
            ..base.clone()
        };
        assert!(core_affecting_changed(&base, &edited));

        let merged = merge_owned(&base, &edited);
        assert_eq!(merged.geo, edited.geo);

        let mut target = base.clone();
        apply_owned_in_place(&mut target, &edited);
        assert_eq!(target.geo, edited.geo);
    }
}

//! Page "Настройки" (T17B).
//!
//! The page keeps a working copy of [`Settings`], mirrors it into the `SettingsBridge` global and
//! writes it back to the controller with a 500 ms debounce. Pure helpers (validation, enum/string
//! mapping, core-affecting comparison) are unit-tested at the bottom.

use crate::controller::Controller;
use crate::{AppWindow, SettingsBridge, Theme};
use flamix_core::model::{ConnectionMode, Dns, Settings, Theme as SettingsTheme};
use slint::{ComponentHandle, Timer, TimerMode};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

/// Debounce before an edit is persisted / applied to the running core.
const SAVE_DEBOUNCE: Duration = Duration::from_millis(500);
/// How long "Сохранено" stays on screen.
const STATUS_TIMEOUT: Duration = Duration::from_secs(2);

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

    let src = settings.clone();
    ctrl.mutate(move |state| apply_owned_in_place(&mut state.settings, &src));
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
    ui.global::<Theme>().set_dark(theme_is_dark(theme));
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
    bridge.set_mux(s.mux);
    bridge.set_fingerprint(s.default_fingerprint.clone().into());
    bridge.set_tun_stack(s.tun_stack.clone().into());
    bridge.set_tun_mtu(s.tun_mtu as i32);
    bridge.set_tun_ipv6(s.tun_ipv6);
    bridge.set_dns_remote(s.dns.remote.clone().into());
    bridge.set_dns_direct(s.dns.direct.clone().into());
    bridge.set_dns_fake_ip(s.dns.fake_ip);
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
        routing: base.routing.clone(),
        failover: base.failover.clone(),
        dns: Dns {
            remote: bridge.get_dns_remote().to_string(),
            direct: bridge.get_dns_direct().to_string(),
            fake_ip: bridge.get_dns_fake_ip(),
            ..base.dns.clone()
        },
        tun_mtu: bridge.get_tun_mtu().max(0) as u32,
        tun_stack: bridge.get_tun_stack().to_string(),
        tun_ipv6: bridge.get_tun_ipv6(),
        mux: bridge.get_mux(),
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
        ..base.clone()
    }
}

/// Inline validation errors; empty string means "no error".
#[derive(Debug, Default, PartialEq, Eq)]
struct Errors {
    ports: String,
    dns: String,
    ping: String,
}

impl Errors {
    fn is_empty(&self) -> bool {
        self.ports.is_empty() && self.dns.is_empty() && self.ping.is_empty()
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
    if !bad_dns.is_empty() {
        errors.dns = format!("Некорректный {} DNS", bad_dns.join(" и "));
    }

    if s.ping_method == "url" && !is_valid_url(&s.ping_url) {
        errors.ping = "Укажите корректный URL (http:// или https://)".into();
    }
    errors
}

fn publish_errors(bridge: &SettingsBridge, errors: &Errors) {
    bridge.set_ports_error(errors.ports.clone().into());
    bridge.set_dns_error(errors.dns.clone().into());
    bridge.set_ping_error(errors.ping.clone().into());
}

/// True when a change must be applied by restarting the running core.
fn core_affecting_changed(a: &Settings, b: &Settings) -> bool {
    a.mode != b.mode
        || a.socks_port != b.socks_port
        || a.http_port != b.http_port
        || a.allow_lan != b.allow_lan
        || a.set_system_proxy != b.set_system_proxy
        || a.proxy_bypass != b.proxy_bypass
        || a.dns != b.dns
        || a.tun_mtu != b.tun_mtu
        || a.tun_stack != b.tun_stack
        || a.tun_ipv6 != b.tun_ipv6
        || a.mux != b.mux
        || a.default_fingerprint != b.default_fingerprint
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

fn bypass_to_text(list: &[String]) -> String {
    list.join("\n")
}

fn bypass_from_text(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect()
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
}

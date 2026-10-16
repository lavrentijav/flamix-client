//! "Статистика" page (page 4).
//!
//! Shows lifetime / today / 30-day totals, a 30-day stacked bar chart, the servers with the
//! most locally accounted traffic and the subscription quotas. The page only talks to Slint
//! through [`StatsBridge`] (see `ui/pages/stats-page.slint`); every value is derived from
//! [`AppState`] and refreshed by a two-second timer while the page is visible.
//!
//! NOTE: [`Server::local_traffic`](flamix_core::model::Server) is cumulative and never reset,
//! so the first card is labelled "Всего" (lifetime), not "Сессия". A true session counter would
//! need the live connection totals; the controller does not expose them publicly yet (follow-up).

use crate::controller::Controller;
use crate::view;
use crate::{AppWindow, DayBar, StatCard, StatsBridge, SubStat, TopServer};
use flamix_core::model::{AppState, TrafficInfo};
use slint::{ComponentHandle, ModelRc, VecModel};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Days in the chart and in the "last 30 days" total.
pub const CHART_DAYS: usize = 30;
/// How many servers the "top servers" card shows.
pub const TOP_SERVERS: usize = 6;
/// How many subscriptions the summary lists.
pub const MAX_SUBS: usize = 12;
/// Refresh cadence while the page is visible.
const REFRESH_INTERVAL: Duration = Duration::from_secs(2);
/// How often the timer checks the current page; short enough to react instantly on entry.
const POLL_INTERVAL: Duration = Duration::from_millis(300);
/// `AppWindow.page` value of this page.
const PAGE_INDEX: i32 = 4;

thread_local! {
    /// Keeps the repeating timer alive for the lifetime of the process.
    static TIMER: RefCell<Option<slint::Timer>> = const { RefCell::new(None) };
}

// --- pure helpers (unit-tested) -------------------------------------------------

/// Sum upload/download of many [`TrafficInfo`]s; quota/expiry are not accumulated.
pub fn sum_infos<'a>(infos: impl IntoIterator<Item = &'a TrafficInfo>) -> TrafficInfo {
    let mut out = TrafficInfo::default();
    for info in infos {
        out.upload = out.upload.saturating_add(info.upload);
        out.download = out.download.saturating_add(info.download);
    }
    out
}

/// The last `days` calendar days ending at `today` (inclusive), oldest first.
/// Days missing from `daily` are zero-filled so the chart always has `days` columns.
pub fn calendar_days(
    daily: &BTreeMap<String, TrafficInfo>,
    today: chrono::NaiveDate,
    days: usize,
) -> Vec<(chrono::NaiveDate, TrafficInfo)> {
    let mut out = Vec::with_capacity(days);
    for offset in (0..days).rev() {
        let date = today
            .checked_sub_days(chrono::Days::new(offset as u64))
            .unwrap_or(today);
        let key = date.format("%Y-%m-%d").to_string();
        out.push((date, daily.get(&key).copied().unwrap_or_default()));
    }
    out
}

/// Bar fractions (upload, download), each relative to the busiest day's total, so the stacked
/// columns stay proportional and the tallest one fills the chart (values are `0.0..=1.0`).
pub fn bar_fractions(days: &[TrafficInfo]) -> Vec<(f32, f32)> {
    let max = days.iter().map(TrafficInfo::used).max().unwrap_or(0).max(1) as f64;
    days.iter()
        .map(|d| {
            (
                (d.upload as f64 / max) as f32,
                (d.download as f64 / max) as f32,
            )
        })
        .collect()
}

/// Summary shown in the three header cards.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Totals {
    /// Lifetime total this client transferred locally (sum of every server's `local_traffic`).
    pub total: TrafficInfo,
    /// The current day's counter.
    pub today: TrafficInfo,
    /// The sum over the last [`CHART_DAYS`] days.
    pub month: TrafficInfo,
}

/// Compute the header totals from the persisted state.
pub fn header_totals(state: &AppState, today: chrono::NaiveDate) -> Totals {
    let total = sum_infos(state.servers().map(|s| &s.local_traffic));
    let key = today.format("%Y-%m-%d").to_string();
    let today_info = state.daily_traffic.get(&key).copied().unwrap_or_default();
    let month = sum_infos(
        calendar_days(&state.daily_traffic, today, CHART_DAYS)
            .iter()
            .map(|(_, info)| info),
    );
    Totals {
        total,
        today: today_info,
        month,
    }
}

/// One row of the "top servers" card.
#[derive(Debug, Clone, PartialEq)]
pub struct TopEntry {
    pub name: String,
    pub iso: String,
    pub used: u64,
    /// Share of the busiest server (`0.0..=1.0`).
    pub fraction: f32,
}

/// Servers with local traffic, busiest first, at most `limit`.
pub fn top_servers(state: &AppState, limit: usize) -> Vec<TopEntry> {
    let mut servers: Vec<_> = state.servers().collect();
    servers.sort_by_key(|s| std::cmp::Reverse(s.local_traffic.used()));
    let max = servers
        .first()
        .map(|s| s.local_traffic.used())
        .unwrap_or(0)
        .max(1);
    servers
        .into_iter()
        .filter(|s| s.local_traffic.used() > 0)
        .take(limit)
        .map(|s| TopEntry {
            name: s.name.clone(),
            iso: view::server_iso(s),
            used: s.local_traffic.used(),
            fraction: s.local_traffic.used() as f32 / max as f32,
        })
        .collect()
}

/// One row of the subscription summary.
#[derive(Debug, Clone, PartialEq)]
pub struct SubEntry {
    pub name: String,
    pub traffic: String,
    pub fraction: f32,
    pub expire: String,
    /// Servers of this subscription that carry their own quota.
    pub quotas: usize,
}

/// Summary of the first `limit` subscriptions.
pub fn subscription_stats(state: &AppState, limit: usize) -> Vec<SubEntry> {
    state
        .subscriptions
        .iter()
        .take(limit)
        .map(|sub| {
            let quotas = sub
                .servers
                .iter()
                .filter(|s| s.traffic.and_then(|t| t.remaining()).is_some())
                .count();
            match sub.traffic {
                Some(info) => SubEntry {
                    name: sub.name.clone(),
                    traffic: view::traffic_text(&info),
                    fraction: view::traffic_fraction(&info),
                    expire: view::expire_text(&info),
                    quotas,
                },
                None => SubEntry {
                    name: sub.name.clone(),
                    traffic: "∞".to_string(),
                    fraction: 0.0,
                    expire: String::new(),
                    quotas,
                },
            }
        })
        .collect()
}

// --- Slint mapping --------------------------------------------------------------

fn model<T: Clone + 'static>(items: Vec<T>) -> ModelRc<T> {
    ModelRc::new(VecModel::from(items))
}

/// One header card from a traffic counter.
pub fn stat_card(title: &str, info: &TrafficInfo, highlight: bool) -> StatCard {
    StatCard {
        title: title.into(),
        value: view::human_bytes(info.used()).into(),
        up: format!("↑ {}", view::human_bytes(info.upload)).into(),
        down: format!("↓ {}", view::human_bytes(info.download)).into(),
        highlight,
    }
}

pub fn build_cards(totals: &Totals) -> Vec<StatCard> {
    vec![
        stat_card("Всего", &totals.total, false),
        stat_card("Сегодня", &totals.today, true),
        stat_card("30 дней", &totals.month, false),
    ]
}

pub fn build_bars(
    days: &[(chrono::NaiveDate, TrafficInfo)],
    fractions: &[(f32, f32)],
) -> Vec<DayBar> {
    days.iter()
        .zip(fractions)
        .map(|((date, info), (up, down))| DayBar {
            label: date.format("%d.%m").to_string().into(),
            day: date.format("%d.%m.%Y").to_string().into(),
            up: view::human_bytes(info.upload).into(),
            down: view::human_bytes(info.download).into(),
            up_fraction: *up,
            down_fraction: *down,
        })
        .collect()
}

pub fn build_top(entries: &[TopEntry]) -> Vec<TopServer> {
    entries
        .iter()
        .map(|e| TopServer {
            name: e.name.clone().into(),
            flag: crate::flags::flag(&e.iso),
            total: view::human_bytes(e.used).into(),
            fraction: e.fraction,
        })
        .collect()
}

pub fn build_subs(entries: &[SubEntry]) -> Vec<SubStat> {
    entries
        .iter()
        .map(|e| SubStat {
            name: e.name.clone().into(),
            traffic: e.traffic.clone().into(),
            fraction: e.fraction,
            expire: e.expire.clone().into(),
            quotas: e.quotas as i32,
        })
        .collect()
}

// --- wiring ---------------------------------------------------------------------

/// Recompute every value and push it to the bridge. Must run on the UI thread.
fn refresh_page(ui: &AppWindow, ctrl: &Arc<Controller>) {
    let today = chrono::Local::now().date_naive();
    let (cards, bars, tops, subs) = ctrl.with_state(|state| {
        let days = calendar_days(&state.daily_traffic, today, CHART_DAYS);
        let infos: Vec<TrafficInfo> = days.iter().map(|(_, info)| *info).collect();
        (
            build_cards(&header_totals(state, today)),
            build_bars(&days, &bar_fractions(&infos)),
            build_top(&top_servers(state, TOP_SERVERS)),
            build_subs(&subscription_stats(state, MAX_SUBS)),
        )
    });
    let bridge = ui.global::<StatsBridge>();
    bridge.set_cards(model(cards));
    bridge.set_bars(model(bars));
    bridge.set_top_servers(model(tops));
    bridge.set_subscriptions(model(subs));
}

/// Clear `daily_traffic` and every server's `local_traffic`.
fn reset_stats(ctrl: &Arc<Controller>) {
    ctrl.mutate(|state| {
        state.daily_traffic.clear();
        let nested = state
            .subscriptions
            .iter_mut()
            .flat_map(|sub| sub.servers.iter_mut());
        let local = state.local_servers.iter_mut();
        for server in nested.chain(local) {
            server.local_traffic = TrafficInfo::default();
        }
    });
    ctrl.status("Статистика сброшена");
}

/// Wire the page. `ctrl == None` means demo / screenshot mode: fill realistic sample data.
pub fn init(ui: &AppWindow, ctrl: Option<&Arc<Controller>>) {
    let Some(ctrl) = ctrl else {
        fill_demo(ui);
        return;
    };
    let ctrl = Arc::clone(ctrl);
    let reset_ctrl = Arc::clone(&ctrl);
    let weak = ui.as_weak();
    ui.global::<StatsBridge>().on_reset_requested(move || {
        reset_stats(&reset_ctrl);
        if let Some(ui) = weak.upgrade() {
            refresh_page(&ui, &reset_ctrl);
        }
    });

    let ui_weak = ui.as_weak();
    let ctrl_timer = Arc::clone(&ctrl);
    // Refresh on the page-entry edge (so a freshly opened page is never empty/stale) and then
    // every REFRESH_INTERVAL while the page stays visible.
    let mut was_visible = ui.get_page() == PAGE_INDEX;
    let mut last_refresh: Option<Instant> = None;
    let timer = slint::Timer::default();
    timer.start(slint::TimerMode::Repeated, POLL_INTERVAL, move || {
        let Some(ui) = ui_weak.upgrade() else {
            return;
        };
        let visible = ui.get_page() == PAGE_INDEX;
        let due = last_refresh.is_none_or(|t| t.elapsed() >= REFRESH_INTERVAL);
        if visible && (!was_visible || due) {
            refresh_page(&ui, &ctrl_timer);
            last_refresh = Some(Instant::now());
        }
        was_visible = visible;
    });
    TIMER.with(|slot| *slot.borrow_mut() = Some(timer));

    // Fill the page once at startup so the first visit shows data immediately.
    refresh_page(ui, &ctrl);
}

// --- demo data ------------------------------------------------------------------

fn demo_server(id: &str, name: &str, iso: &str, up: u64, down: u64) -> flamix_core::model::Server {
    use flamix_core::model::{Credentials, Security, Server};
    Server {
        id: id.to_string(),
        name: name.to_string(),
        country: Some(iso.to_string()),
        address: "example.com".to_string(),
        port: 443,
        credentials: Credentials::Trojan {
            password: "demo".to_string(),
        },
        transport: Default::default(),
        security: Security::None,
        raw_link: None,
        traffic: None,
        local_traffic: TrafficInfo {
            upload: up,
            download: down,
            ..TrafficInfo::default()
        },
        last_ping_ms: None,
    }
}

fn demo_subscription(
    id: &str,
    name: &str,
    url: &str,
    servers: Vec<flamix_core::model::Server>,
) -> flamix_core::model::Subscription {
    use flamix_core::model::Subscription;
    Subscription {
        id: id.to_string(),
        name: name.to_string(),
        auto_title: None,
        url: url.to_string(),
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
        servers,
        provider: Default::default(),
        options: Default::default(),
    }
}

/// A realistic demo state used for screenshots (`--demo`).
fn demo_state(today: chrono::NaiveDate) -> AppState {
    let mut daily = BTreeMap::new();
    for offset in 0..CHART_DAYS {
        let date = today
            .checked_sub_days(chrono::Days::new(offset as u64))
            .unwrap_or(today);
        let i = offset as u64;
        let up = (60 + (i * 53) % 260) * 1024 * 1024 / 4;
        let down = (600 + (i * 137) % 3800) * 1024 * 1024;
        daily.insert(
            date.format("%Y-%m-%d").to_string(),
            TrafficInfo {
                upload: up,
                download: down,
                ..TrafficInfo::default()
            },
        );
    }

    let mut premium = demo_subscription(
        "sub-premium",
        "Flamix Premium",
        "https://example.com/sub",
        vec![
            demo_server(
                "de-1",
                "Франкфурт · Reality",
                "DE",
                2_100_000_000,
                18_400_000_000,
            ),
            demo_server("nl-1", "Амстердам · gRPC", "NL", 900_000_000, 7_600_000_000),
            demo_server(
                "fi-1",
                "Хельсинки · xhttp",
                "FI",
                300_000_000,
                2_100_000_000,
            ),
        ],
    );
    premium.traffic = Some(TrafficInfo {
        upload: 4_000_000_000,
        download: 26_000_000_000,
        total: Some(100 * 1024 * 1024 * 1024),
        expire: Some(1_798_761_600),
    });
    premium.servers[0].traffic = Some(TrafficInfo {
        upload: 1_000_000_000,
        download: 5_000_000_000,
        total: Some(20 * 1024 * 1024 * 1024),
        expire: None,
    });

    let basic = demo_subscription(
        "sub-basic",
        "Базовый тариф",
        "https://example.com/basic",
        vec![
            demo_server("us-1", "Нью-Йорк", "US", 120_000_000, 1_400_000_000),
            demo_server("ru-1", "Москва · WS", "RU", 40_000_000, 380_000_000),
        ],
    );

    AppState {
        subscriptions: vec![premium, basic],
        local_servers: vec![demo_server(
            "local-1",
            "Локальный сервер",
            "RU",
            20_000_000,
            160_000_000,
        )],
        daily_traffic: daily,
        ..AppState::default()
    }
}

fn fill_demo(ui: &AppWindow) {
    let today = chrono::Local::now().date_naive();
    let state = demo_state(today);
    let days = calendar_days(&state.daily_traffic, today, CHART_DAYS);
    let infos: Vec<TrafficInfo> = days.iter().map(|(_, info)| *info).collect();
    let bridge = ui.global::<StatsBridge>();
    bridge.set_cards(model(build_cards(&header_totals(&state, today))));
    bridge.set_bars(model(build_bars(&days, &bar_fractions(&infos))));
    bridge.set_top_servers(model(build_top(&top_servers(&state, TOP_SERVERS))));
    bridge.set_subscriptions(model(build_subs(&subscription_stats(&state, MAX_SUBS))));
}

#[cfg(test)]
mod tests {
    use super::*;
    use flamix_core::model::{Credentials, Security, Server, Subscription};

    fn date(y: i32, m: u32, d: u32) -> chrono::NaiveDate {
        chrono::NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    fn info(up: u64, down: u64) -> TrafficInfo {
        TrafficInfo {
            upload: up,
            download: down,
            ..TrafficInfo::default()
        }
    }

    fn server(id: &str, name: &str, iso: &str, up: u64, down: u64) -> Server {
        Server {
            id: id.to_string(),
            name: name.to_string(),
            country: Some(iso.to_string()),
            address: "example.com".to_string(),
            port: 443,
            credentials: Credentials::Trojan {
                password: "p".to_string(),
            },
            transport: Default::default(),
            security: Security::None,
            raw_link: None,
            traffic: None,
            local_traffic: info(up, down),
            last_ping_ms: None,
        }
    }

    fn subscription(id: &str, name: &str, servers: Vec<Server>) -> Subscription {
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
            servers,
            provider: Default::default(),
            options: Default::default(),
        }
    }

    #[test]
    fn sum_infos_adds_upload_and_download_saturating() {
        let total = sum_infos([info(1, 2), info(3, 4)].iter());
        assert_eq!(total, info(4, 6));
        assert_eq!(sum_infos(std::iter::empty()), TrafficInfo::default());
        let big = sum_infos([info(u64::MAX, u64::MAX), info(5, 5)].iter());
        assert_eq!(big.upload, u64::MAX);
        assert_eq!(big.download, u64::MAX);
    }

    #[test]
    fn calendar_days_zero_fills_and_orders_oldest_first() {
        let today = date(2026, 5, 10);
        let mut daily = BTreeMap::new();
        daily.insert("2026-05-10".to_string(), info(1, 1));
        daily.insert("2026-05-08".to_string(), info(2, 2));
        daily.insert("2026-01-01".to_string(), info(9, 9)); // outside the window
        let days = calendar_days(&daily, today, 3);
        assert_eq!(days.len(), 3);
        assert_eq!(days[0].0, date(2026, 5, 8));
        assert_eq!(days[0].1, info(2, 2));
        assert_eq!(days[1].0, date(2026, 5, 9));
        assert_eq!(days[1].1, TrafficInfo::default());
        assert_eq!(days[2].0, date(2026, 5, 10));
        assert_eq!(days[2].1, info(1, 1));
    }

    #[test]
    fn bar_fractions_scale_to_the_busiest_day() {
        let fractions = bar_fractions(&[info(50, 50), info(0, 0), info(25, 25)]);
        assert_eq!(fractions[0], (0.5, 0.5));
        assert_eq!(fractions[1], (0.0, 0.0));
        assert_eq!(fractions[2], (0.25, 0.25));
    }

    #[test]
    fn bar_fractions_handles_all_zero() {
        let fractions = bar_fractions(&[info(0, 0), info(0, 0)]);
        assert_eq!(fractions, vec![(0.0, 0.0), (0.0, 0.0)]);
    }

    #[test]
    fn header_totals_total_today_and_month() {
        let today = date(2026, 5, 10);
        let mut daily = BTreeMap::new();
        daily.insert("2026-05-10".to_string(), info(10, 90));
        daily.insert("2026-05-09".to_string(), info(1, 9));
        daily.insert("2026-04-01".to_string(), info(100, 100)); // outside 30 days
        let state = AppState {
            subscriptions: vec![subscription("s", "S", vec![server("a", "A", "DE", 2, 8)])],
            local_servers: vec![server("l", "L", "RU", 1, 1)],
            daily_traffic: daily,
            ..AppState::default()
        };
        let totals = header_totals(&state, today);
        assert_eq!(totals.total, info(3, 9));
        assert_eq!(totals.today, info(10, 90));
        // 30-day window includes May 10 and May 9, excludes April 1.
        assert_eq!(totals.month, info(11, 99));
    }

    #[test]
    fn top_servers_sorted_and_scaled_skipping_idle() {
        let state = AppState {
            subscriptions: vec![subscription(
                "s",
                "S",
                vec![
                    server("a", "Fast", "DE", 10, 90),
                    server("b", "Slow", "NL", 1, 9),
                    server("c", "Idle", "FI", 0, 0),
                ],
            )],
            ..AppState::default()
        };
        let top = top_servers(&state, 5);
        assert_eq!(top.len(), 2);
        assert_eq!(top[0].name, "Fast");
        assert_eq!(top[0].iso, "DE");
        assert_eq!(top[0].used, 100);
        assert!((top[0].fraction - 1.0).abs() < 1e-6);
        assert_eq!(top[1].name, "Slow");
        assert!((top[1].fraction - 0.1).abs() < 1e-6);
    }

    #[test]
    fn top_servers_respects_limit() {
        let state = AppState {
            subscriptions: vec![subscription(
                "s",
                "S",
                vec![
                    server("a", "A", "DE", 1, 1),
                    server("b", "B", "NL", 2, 2),
                    server("c", "C", "FI", 3, 3),
                ],
            )],
            ..AppState::default()
        };
        assert_eq!(top_servers(&state, 2).len(), 2);
    }

    #[test]
    fn subscription_stats_reports_quota_expiry_and_counts() {
        let mut sub = subscription(
            "s",
            "Premium",
            vec![server("a", "A", "DE", 0, 0), server("b", "B", "NL", 0, 0)],
        );
        sub.traffic = Some(TrafficInfo {
            upload: 5,
            download: 15,
            total: Some(100),
            expire: Some(0),
        });
        // Server "a" has a real limit (counted); server "b" reports traffic but no limit and
        // must NOT be counted as a quota.
        sub.servers[0].traffic = Some(TrafficInfo {
            upload: 1,
            download: 1,
            total: Some(10),
            expire: None,
        });
        sub.servers[1].traffic = Some(info(0, 0));
        let limited = subscription("s2", "Basic", vec![server("c", "C", "FI", 0, 0)]);
        let state = AppState {
            subscriptions: vec![sub, limited],
            ..AppState::default()
        };
        let stats = subscription_stats(&state, 10);
        assert_eq!(stats.len(), 2);
        assert_eq!(stats[0].name, "Premium");
        assert_eq!(stats[0].traffic, "20 B / 100 B");
        assert!((stats[0].fraction - 0.2).abs() < 1e-6);
        assert_eq!(stats[0].quotas, 1);
        assert_eq!(stats[1].traffic, "∞");
        assert_eq!(stats[1].quotas, 0);
    }

    #[test]
    fn build_top_maps_entries() {
        let entries = vec![
            TopEntry {
                name: "Fast".to_string(),
                iso: "DE".to_string(),
                used: 1024 * 1024,
                fraction: 1.0,
            },
            TopEntry {
                name: "Slow".to_string(),
                iso: "ru".to_string(),
                used: 512,
                fraction: 0.5,
            },
        ];
        let rows = build_top(&entries);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].name, "Fast");
        assert_eq!(rows[0].total, "1.0 MB");
        assert!((rows[0].fraction - 1.0).abs() < 1e-6);
        assert_eq!(rows[1].total, "512 B");
        assert!((rows[1].fraction - 0.5).abs() < 1e-6);
    }

    #[test]
    fn build_subs_maps_entries() {
        let entries = vec![
            SubEntry {
                name: "Premium".to_string(),
                traffic: "20 B / 100 B".to_string(),
                fraction: 0.2,
                expire: "до 01.01.2027".to_string(),
                quotas: 3,
            },
            SubEntry {
                name: "Basic".to_string(),
                traffic: "∞".to_string(),
                fraction: 0.0,
                expire: String::new(),
                quotas: 0,
            },
        ];
        let rows = build_subs(&entries);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].name, "Premium");
        assert_eq!(rows[0].traffic, "20 B / 100 B");
        assert!((rows[0].fraction - 0.2).abs() < 1e-6);
        assert_eq!(rows[0].expire, "до 01.01.2027");
        assert_eq!(rows[0].quotas, 3);
        assert_eq!(rows[1].quotas, 0);
    }

    #[test]
    fn stat_card_formats_bytes() {
        let card = stat_card("Сегодня", &info(1024, 1024 * 1024), true);
        assert_eq!(card.title, "Сегодня");
        assert_eq!(card.value, "1.0 MB");
        assert_eq!(card.up, "↑ 1.0 KB");
        assert_eq!(card.down, "↓ 1.0 MB");
        assert!(card.highlight);
    }

    #[test]
    fn build_bars_has_one_bar_per_day() {
        let days = vec![
            (date(2026, 5, 9), info(0, 0)),
            (date(2026, 5, 10), info(10, 10)),
        ];
        let bars = build_bars(&days, &bar_fractions(&[info(0, 0), info(10, 10)]));
        assert_eq!(bars.len(), 2);
        assert_eq!(bars[1].label, "10.05");
        assert_eq!(bars[1].day, "10.05.2026");
        assert_eq!(bars[1].down_fraction, 0.5);
    }
}

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

use flamix_core::model::{AppState, Subscription};
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

/// Pushes the subscription cards and the selection from `state` into the window.
fn refresh(window: &AndroidWindow, state: &AppState, shared: &Shared) {
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
            let id = id.to_string();
            let Ok(state) = store::load(&dir) else { return };
            let Some(sub) = state.subscriptions.iter().find(|s| s.id == id).cloned() else {
                return;
            };
            {
                let mut ui = lock(&shared);
                if ui.updating.contains(&id) {
                    return;
                }
                ui.updating.push(id.clone());
            }
            refresh(&w, &state, &shared);
            let hwid = state.effective_hwid();
            let weak = weak.clone();
            let dir = dir.clone();
            let shared = shared.clone();
            let shared_err = shared.clone();
            let dir_err = dir.clone();
            let id_thread = id.clone();
            // Network fetch on a plain thread; the result is stored and shown on the UI thread.
            let spawned = std::thread::Builder::new()
                .name("subscription-refresh".into())
                .spawn(move || {
                    let mut sub = sub;
                    let fetched = phone::fetch_subscription(&mut sub, hwid);
                    let _ = slint::invoke_from_event_loop(move || {
                        lock(&shared).updating.retain(|x| *x != id_thread);
                        let Some(w) = weak.upgrade() else { return };
                        match fetched {
                            Ok(()) => show(&w, &shared, phone::apply_fetched(&dir, sub)),
                            Err(err) => {
                                // The fetch error is kept on the subscription (last_error).
                                let _ = phone::apply_fetched(&dir, sub);
                                w.set_status_text(format!("Ошибка обновления: {err}").into());
                                reload(&w, &dir, &shared);
                            }
                        }
                    });
                });
            if let Err(err) = spawned {
                lock(&shared_err).updating.retain(|x| *x != id);
                w.set_status_text(format!("Не удалось запустить обновление: {err}").into());
                reload(&w, &dir_err, &shared_err);
            }
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
        move |url| {
            let Some(w) = weak.upgrade() else { return };
            let mut sub = match phone::new_subscription(&url) {
                Ok(sub) => sub,
                Err(err) => {
                    w.set_add_status(err.into());
                    return;
                }
            };
            w.set_add_status("Загрузка подписки…".into());
            let hwid = store::load(&dir).ok().and_then(|s| s.effective_hwid());
            let weak = weak.clone();
            let dir = dir.clone();
            let shared = shared.clone();
            // Network fetch on a plain thread; the result is applied on the UI thread.
            let spawned = std::thread::Builder::new()
                .name("subscription".into())
                .spawn(move || {
                    let fetched = phone::fetch_subscription(&mut sub, hwid).map(|()| sub);
                    let _ = slint::invoke_from_event_loop(move || {
                        let Some(w) = weak.upgrade() else { return };
                        match fetched {
                            Ok(sub) => add_fetched(&w, &dir, &shared, sub),
                            Err(err) => w.set_add_status(format!("Ошибка: {err}").into()),
                        }
                    });
                });
            if let Err(err) = spawned {
                w.set_add_status(format!("Не удалось запустить загрузку: {err}").into());
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

/// Saves a fetched subscription and shows the result (UI thread).
fn add_fetched(window: &AndroidWindow, dir: &Path, shared: &Shared, sub: Subscription) {
    match phone::add_subscription(dir, sub) {
        Ok(state) => {
            refresh(window, &state, shared);
            window.set_add_status("Подписка добавлена".into());
        }
        Err(err) => window.set_add_status(err.into()),
    }
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

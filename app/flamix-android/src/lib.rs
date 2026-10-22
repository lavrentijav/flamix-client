//! Android entry point for Flamix (M4: phone layout).
//!
//! Kept in its own crate so the Android build does not pull in the desktop-only
//! dependencies of `app/flamix` (tray, native file dialogs, clipboard, ...).
//!
//! The window is `AndroidWindow` from `ui/android.slint`. State lives in `state.json` in the app's
//! files dir (same format as the desktop app); see [`phone`] for the logic.

#[cfg(unix)]
pub mod applog;
pub mod bridge;
pub mod phone;
#[cfg(unix)]
pub mod tunnel;

slint::include_modules!();

use flamix_core::model::{AppState, Subscription};
use flamix_core::store;
use slint::{ComponentHandle, ModelRc, Timer, TimerMode, VecModel};
use std::path::{Path, PathBuf};
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

/// Pushes the server list and the selection from `state` into the window.
fn refresh(window: &AndroidWindow, state: &AppState) {
    let selected = state.selected_server.as_deref();
    let items: Vec<ServerItem> = phone::server_rows(state)
        .into_iter()
        .map(|row| ServerItem {
            selected: Some(row.id.as_str()) == selected,
            name: row.name.into(),
            subtitle: row.subtitle.into(),
            address: row.address.into(),
            is_exit: row.is_exit,
            group: i32::try_from(row.group).unwrap_or(0),
        })
        .collect();
    window.set_servers(ModelRc::new(VecModel::from(items)));
    window.set_selected_name(phone::selected_name(state).into());
}

/// Loads the state for the first screen and wires the callbacks. Blocks in the Slint event loop.
pub fn run(data_dir: PathBuf) -> Result<(), slint::PlatformError> {
    let window = AndroidWindow::new()?;
    match store::load(&data_dir) {
        Ok(state) => refresh(&window, &state),
        Err(err) => window.set_status_text(format!("Не удалось прочитать данные: {err}").into()),
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
        move |index| {
            let Some(w) = weak.upgrade() else { return };
            let Ok(index) = usize::try_from(index) else {
                return;
            };
            match phone::update_state(&dir, |st| {
                st.selected_server = phone::server_id_at(st, index)
            }) {
                Ok(state) => {
                    refresh(&w, &state);
                    w.set_status_text("".into());
                }
                Err(err) => w.set_status_text(err.into()),
            }
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
            // Network fetch on a plain thread; the result is applied on the UI thread.
            let spawned = std::thread::Builder::new()
                .name("subscription".into())
                .spawn(move || {
                    let fetched = phone::fetch_subscription(&mut sub, hwid).map(|()| sub);
                    let _ = slint::invoke_from_event_loop(move || {
                        let Some(w) = weak.upgrade() else { return };
                        match fetched {
                            Ok(sub) => add_fetched(&w, &dir, sub),
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
fn add_fetched(window: &AndroidWindow, dir: &Path, sub: Subscription) {
    match phone::add_subscription(dir, sub) {
        Ok(state) => {
            refresh(window, &state);
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
    match slint::android::init(app) {
        Ok(()) => {
            if let Err(e) = run(data_dir) {
                applog::log("E", "android_main", &format!("ui exited with error: {e}"));
            }
        }
        Err(e) => applog::log("E", "android_main", &format!("slint init failed: {e}")),
    }
}

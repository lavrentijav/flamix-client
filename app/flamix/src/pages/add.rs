//! Page "Добавить" (task T17A). Talks to the page only through the `AddBridge` global.
//!
//! The subscription form is applied in two steps: [`Controller::import`] parses the URL and adds
//! the subscription (and starts its fetch), then we fill the optional name / backup URLs by
//! matching it by id. Because the fetch and the meta write race, a bounded background task
//! re-applies the meta once the first fetch finishes (see [`add_subscription`]).

use crate::controller::Controller;
use crate::{AddBridge, AppWindow};
use flamix_core::model::AppState;
use flamix_core::{deeplink, link};
use slint::{ComponentHandle, Weak};
use std::sync::Arc;
use std::time::Duration;

/// Poll interval / tick budget while waiting for the first subscription fetch to finish.
const META_REAPPLY_TICK: Duration = Duration::from_millis(250);
const META_REAPPLY_TICKS: u32 = 240;

/// Demo contents for `--demo` / screenshot mode.
const DEMO_URL: &str = "https://sub.flamix.example/api/v1/client/subscribe?token=8f3c1a";
const DEMO_NAME: &str = "Flamix Premium";
const DEMO_BACKUPS: &str = "https://backup.flamix.example/sub\nhttps://eu.flamix.example/sub";
const DEMO_LINKS: &str = "vless://24c9d3a1-7b6e-4f2a-9c10@de1.flamix.example:443?type=tcp&security=reality#Frankfurt · Reality\nvmess://eyJhZGQiOiJubDEuZmxhbWl4LmV4YW1wbGUifQ==\ntrojan://secret@sg1.flamix.example:443#Singapore";

pub fn init(ui: &AppWindow, ctrl: Option<&Arc<Controller>>) {
    match ctrl {
        Some(ctrl) => wire(ui, Arc::clone(ctrl)),
        None => fill_demo(ui),
    }
}

fn fill_demo(ui: &AppWindow) {
    let bridge = ui.global::<AddBridge>();
    bridge.set_url(DEMO_URL.into());
    bridge.set_name(DEMO_NAME.into());
    bridge.set_backups(DEMO_BACKUPS.into());
    bridge.set_links(DEMO_LINKS.into());
    bridge.set_status("Готово к добавлению".into());
}

fn wire(ui: &AppWindow, ctrl: Arc<Controller>) {
    {
        let ctrl = Arc::clone(&ctrl);
        let weak = ui.as_weak();
        ui.global::<AddBridge>()
            .on_add_subscription(move || add_subscription(weak.clone(), ctrl.clone()));
    }
    {
        let ctrl = Arc::clone(&ctrl);
        let weak = ui.as_weak();
        ui.global::<AddBridge>()
            .on_import_links(move || import_links(weak.clone(), ctrl.clone()));
    }
    {
        let ctrl = Arc::clone(&ctrl);
        let weak = ui.as_weak();
        ui.global::<AddBridge>()
            .on_import_clipboard(move || import_clipboard(weak.clone(), ctrl.clone()));
    }
}

/// "Add" button: import a subscription URL, then apply the optional name / backup URLs.
///
/// [`Controller::import`] adds the subscription and immediately spawns its fetch, which clones
/// the subscription before `subscription::update` rewrites it. If that clone happens before our
/// `mutate`, the typed name / backup URLs are lost. The meta is therefore applied as early as
/// possible, and while the first fetch is still pending a bounded background task re-applies it
/// once the fetch finishes (detected through `last_update` / `last_error`).
fn add_subscription(ui: Weak<AppWindow>, ctrl: Arc<Controller>) {
    let Some(window) = ui.upgrade() else {
        return;
    };
    let bridge = window.global::<AddBridge>();
    let url = bridge.get_url().trim().to_string();
    let name = bridge.get_name().trim().to_string();
    let backups = parse_backup_urls(&bridge.get_backups());

    if let Err(message) = validate_subscription_url(&url) {
        set_status(&ui, message);
        return;
    }

    bridge.set_busy(true);
    ctrl.import(&url);
    let id = ctrl.with_state(|state| {
        state
            .subscriptions
            .iter()
            .rev()
            .find(|s| s.url == url)
            .map(|s| s.id.clone())
    });
    let Some(id) = id else {
        bridge.set_busy(false);
        set_status(&ui, "Не удалось распознать ссылку на подписку");
        return;
    };

    // The fetch may still be pending; remember that so we can correct the meta afterwards.
    let pending = ctrl.with_state(|state| {
        state
            .subscriptions
            .iter()
            .find(|s| s.id == id)
            .is_some_and(|s| s.last_update.is_none() && s.last_error.is_none())
    });
    ctrl.mutate(|state| {
        apply_subscription_meta(state, &id, &name, &backups);
    });
    if pending && !(name.is_empty() && backups.is_empty()) {
        schedule_meta_reapply(Arc::clone(&ctrl), id, name, backups);
    }

    bridge.set_status("Подписка добавлена".into());
    bridge.set_url("".into());
    bridge.set_name("".into());
    bridge.set_backups("".into());
    bridge.set_busy(false);
    window.set_page(0);
}

/// Re-apply the user meta after the first fetch; the fetch may have overwritten it.
fn schedule_meta_reapply(ctrl: Arc<Controller>, id: String, name: String, backups: Vec<String>) {
    ctrl.runtime().spawn(async move {
        for _ in 0..META_REAPPLY_TICKS {
            tokio::time::sleep(META_REAPPLY_TICK).await;
            let finished = ctrl.with_state(|state| {
                state
                    .subscriptions
                    .iter()
                    .find(|s| s.id == id)
                    .is_none_or(|sub| sub.last_update.is_some() || sub.last_error.is_some())
            });
            if finished {
                ctrl.mutate(|state| reapply_meta(state, &id, &name, &backups));
                return;
            }
        }
    });
}

/// "Импортировать" button: parse the pasted share links.
fn import_links(ui: Weak<AppWindow>, ctrl: Arc<Controller>) {
    let Some(window) = ui.upgrade() else {
        return;
    };
    let text = window.global::<AddBridge>().get_links().to_string();
    if text.trim().is_empty() {
        return;
    }
    let bridge = window.global::<AddBridge>();
    bridge.set_busy(true);

    let import = match deeplink::parse_import(&text) {
        Some(import) => import,
        None => {
            bridge.set_busy(false);
            set_status(&ui, "Не удалось распознать ссылки");
            return;
        }
    };

    let before = ctrl.with_state(|state| (state.subscriptions.len(), state.local_servers.len()));
    ctrl.import(&text);
    let added = ctrl.with_state(|state| {
        state.subscriptions.len().saturating_sub(before.0)
            + state.local_servers.len().saturating_sub(before.1)
    });

    bridge.set_links("".into());
    bridge.set_busy(false);

    if matches!(import, deeplink::Import::Links(_)) {
        let skipped = link::parse_links(&text, "local").1.len();
        let message = if added == 0 && skipped == 0 {
            "Все серверы уже добавлены".to_string()
        } else {
            links_status(added, skipped)
        };
        set_status(&ui, message);
    }
    if added > 0 {
        window.set_page(0);
    }
}

/// "Из буфера обмена": read the clipboard off-thread and import it (URL, links or deep link).
fn import_clipboard(ui: Weak<AppWindow>, ctrl: Arc<Controller>) {
    if let Some(window) = ui.upgrade() {
        window.global::<AddBridge>().set_busy(true);
    }
    let worker = Arc::clone(&ctrl);
    let result_ui = ui.clone();
    ctrl.runtime().spawn_blocking(move || {
        let text = match arboard::Clipboard::new().and_then(|mut clipboard| clipboard.get_text()) {
            Ok(text) => text,
            Err(_) => {
                finish(&result_ui, "Буфер обмена пуст", false);
                return;
            }
        };
        let recognized = deeplink::parse_import(&text).is_some();
        let before =
            worker.with_state(|state| (state.subscriptions.len(), state.local_servers.len()));
        worker.import(&text);
        let added = worker.with_state(|state| {
            state.subscriptions.len().saturating_sub(before.0)
                + state.local_servers.len().saturating_sub(before.1)
        });
        finish(&result_ui, import_result(added, recognized), added > 0);
    });
}

/// Publish a one-off result: set the status, clear the busy flag and optionally navigate away.
fn finish(ui: &Weak<AppWindow>, text: impl Into<String>, navigate: bool) {
    let text = text.into();
    let _ = ui.upgrade_in_event_loop(move |ui| {
        let bridge = ui.global::<AddBridge>();
        bridge.set_status(text.into());
        bridge.set_busy(false);
        if navigate {
            ui.set_page(0);
        }
    });
}

/// Push the result line to the page from any thread.
fn set_status(ui: &Weak<AppWindow>, text: impl Into<String>) {
    let text = text.into();
    let _ = ui.upgrade_in_event_loop(move |ui| {
        ui.global::<AddBridge>().set_status(text.into());
    });
}

/// Validate the subscription URL field. Empty is handled by the disabled button, but re-checked.
fn validate_subscription_url(url: &str) -> Result<(), String> {
    if url.is_empty() {
        return Err("Введите URL подписки".into());
    }
    if url.chars().any(char::is_whitespace) {
        return Err("URL не должен содержать пробелы".into());
    }
    let lower = url.to_ascii_lowercase();
    if !lower.starts_with("http://") && !lower.starts_with("https://") {
        return Err("URL должен начинаться с http:// или https://".into());
    }
    Ok(())
}

/// One non-empty, trimmed line per backup URL.
fn parse_backup_urls(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect()
}

/// Apply the optional name / backup URLs to the subscription with this id.
fn apply_subscription_meta(state: &mut AppState, id: &str, name: &str, backups: &[String]) -> bool {
    let Some(sub) = state.subscriptions.iter_mut().find(|s| s.id == id) else {
        return false;
    };
    if !name.is_empty() {
        sub.name = name.to_string();
    }
    if !backups.is_empty() {
        sub.backup_urls = backups.to_vec();
    }
    true
}

/// Restore the user name and merge the user backup URLs, keeping any added by the provider.
fn reapply_meta(state: &mut AppState, id: &str, name: &str, backups: &[String]) {
    let Some(sub) = state.subscriptions.iter_mut().find(|s| s.id == id) else {
        return;
    };
    if !name.is_empty() && sub.name != name {
        sub.name = name.to_string();
    }
    for backup in backups {
        if !sub.backup_urls.contains(backup) {
            sub.backup_urls.push(backup.clone());
        }
    }
}

/// Result line for a share-link import (matches the controller's wording, skipped kept).
fn links_status(added: usize, skipped: usize) -> String {
    let mut text = format!("Добавлено серверов: {added}");
    if skipped > 0 {
        text.push_str(&format!(", пропущено: {skipped}"));
    }
    text
}

/// Result line for the generic clipboard import.
fn import_result(added: usize, recognized: bool) -> String {
    if added > 0 {
        format!("Импортировано: {added}")
    } else if recognized {
        "Уже добавлено".to_string()
    } else {
        "Не удалось распознать".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::controller::apply_import;

    fn state_with_subscription(url: &str) -> AppState {
        let mut state = AppState::default();
        apply_import(
            &mut state,
            deeplink::Import::Subscription {
                url: url.to_string(),
                name: None,
            },
        );
        state
    }

    fn only_id(state: &AppState) -> String {
        state.subscriptions[0].id.clone()
    }

    #[test]
    fn validate_subscription_url_accepts_http_and_https() {
        assert!(validate_subscription_url("https://example.com/sub").is_ok());
        assert!(validate_subscription_url("http://example.com/sub").is_ok());
        assert!(validate_subscription_url("HTTPS://Example.com").is_ok());
    }

    #[test]
    fn validate_subscription_url_rejects_bad_input() {
        assert!(validate_subscription_url("").is_err());
        assert!(validate_subscription_url("example.com/sub").is_err());
        assert!(validate_subscription_url("ftp://example.com/sub").is_err());
        assert!(validate_subscription_url("https://exa mple.com").is_err());
    }

    #[test]
    fn parse_backup_urls_trims_and_drops_empty_lines() {
        let parsed = parse_backup_urls("  https://a.example/sub \n\n https://b.example/sub\n  ");
        assert_eq!(
            parsed,
            vec![
                "https://a.example/sub".to_string(),
                "https://b.example/sub".to_string()
            ]
        );
        assert!(parse_backup_urls(" \n  \n ").is_empty());
    }

    #[test]
    fn apply_subscription_meta_sets_name_and_backups() {
        let mut state = state_with_subscription("https://example.com/sub");
        let id = only_id(&state);
        let backups = vec!["https://backup.example.com/sub".to_string()];
        assert!(apply_subscription_meta(&mut state, &id, "My Sub", &backups));
        assert_eq!(state.subscriptions[0].name, "My Sub");
        assert_eq!(state.subscriptions[0].backup_urls, backups);
    }

    #[test]
    fn apply_subscription_meta_keeps_name_when_empty() {
        let mut state = state_with_subscription("https://example.com/sub");
        let id = only_id(&state);
        let original = state.subscriptions[0].name.clone();
        assert!(apply_subscription_meta(&mut state, &id, "", &[]));
        assert_eq!(state.subscriptions[0].name, original);
        assert!(state.subscriptions[0].backup_urls.is_empty());
    }

    #[test]
    fn apply_subscription_meta_reports_missing_id() {
        let mut state = state_with_subscription("https://example.com/sub");
        assert!(!apply_subscription_meta(&mut state, "missing", "N", &[]));
    }

    #[test]
    fn reapply_meta_restores_name_and_merges_backups() {
        let mut state = state_with_subscription("https://example.com/sub");
        let id = only_id(&state);
        // Simulate the fetch having overwritten the meta and added a provider backup.
        state.subscriptions[0].name = "Provider title".to_string();
        state.subscriptions[0].backup_urls = vec!["https://provider.example/sub".to_string()];
        reapply_meta(
            &mut state,
            &id,
            "My Sub",
            &[
                "https://provider.example/sub".to_string(),
                "https://mine.example/sub".to_string(),
            ],
        );
        assert_eq!(state.subscriptions[0].name, "My Sub");
        assert_eq!(
            state.subscriptions[0].backup_urls,
            vec![
                "https://provider.example/sub".to_string(),
                "https://mine.example/sub".to_string()
            ]
        );
    }

    #[test]
    fn reapply_meta_keeps_provider_name_when_no_user_name() {
        let mut state = state_with_subscription("https://example.com/sub");
        let id = only_id(&state);
        state.subscriptions[0].name = "Provider title".to_string();
        reapply_meta(&mut state, &id, "", &[]);
        assert_eq!(state.subscriptions[0].name, "Provider title");
    }

    #[test]
    fn links_status_includes_skipped_only_when_present() {
        assert_eq!(links_status(0, 0), "Добавлено серверов: 0");
        assert_eq!(links_status(3, 0), "Добавлено серверов: 3");
        assert_eq!(links_status(2, 5), "Добавлено серверов: 2, пропущено: 5");
    }

    #[test]
    fn import_result_covers_added_duplicate_and_unknown() {
        assert_eq!(import_result(0, false), "Не удалось распознать");
        assert_eq!(import_result(0, true), "Уже добавлено");
        assert_eq!(import_result(4, true), "Импортировано: 4");
    }
}

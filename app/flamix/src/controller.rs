//! UI controller: owns [`AppState`] and bridges the Slint callbacks to the core/sys crates.
//!
//! The UI thread never does I/O. Every callback either mutates the in-memory state and
//! persists it, or hands work to the multi-thread tokio runtime; background results are pushed
//! back with `Weak::upgrade_in_event_loop`.

use crate::view;
use crate::{flags, AppWindow, ConnState};
use flamix_core::model::{AppState, ConnectionMode, Subscription};
use flamix_core::subscription::{self, FetchOptions};
use flamix_core::{deeplink, link, provider_routing};
use slint::{ComponentHandle, Weak};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Maximum number of subscription fetches running at once.
const MAX_PARALLEL_UPDATES: usize = 3;

/// Interval between the periodic auto-update checks.
const AUTO_UPDATE_INTERVAL: Duration = Duration::from_secs(10 * 60);

/// Everything needed to talk to the running cores (filled in when connected).
pub struct Connection {}

/// Result of a pure [`apply_import`] call, used for the status text and follow-up work.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportOutcome {
    /// A fresh subscription was added (fetch it next).
    SubscriptionAdded { id: String, name: String },
    /// Local servers were added from share links.
    LinksAdded { added: usize, skipped: usize },
    /// Provider routing was applied to this subscription.
    Routing { subscription: String, enabled: bool },
    /// Nothing to apply (routing without a subscription).
    Unsupported,
}

/// Apply one parsed deep link to `state`. Pure: no I/O, so it is unit-tested directly.
pub fn apply_import(state: &mut AppState, import: deeplink::Import) -> ImportOutcome {
    match import {
        deeplink::Import::Subscription { url, name } => {
            let id = random_hex(8);
            let name = name.unwrap_or_default();
            state.subscriptions.push(Subscription {
                id: id.clone(),
                name: name.clone(),
                auto_title: None,
                url,
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
                options: Default::default(),
            });
            ImportOutcome::SubscriptionAdded { id, name }
        }
        deeplink::Import::Links(text) => {
            let (servers, errors) = link::parse_links(&text, "local");
            let mut added = 0usize;
            for server in servers {
                if state.local_servers.iter().any(|s| s.id == server.id) {
                    continue;
                }
                state.local_servers.push(server);
                added += 1;
            }
            ImportOutcome::LinksAdded {
                added,
                skipped: errors.len(),
            }
        }
        deeplink::Import::Routing(action) => apply_routing(state, action),
    }
}

fn apply_routing(state: &mut AppState, action: provider_routing::RoutingLinkAction) -> ImportOutcome {
    let Some(index) = subscription_of_selected(state) else {
        return ImportOutcome::Unsupported;
    };
    let sub = &mut state.subscriptions[index];
    match action {
        provider_routing::RoutingLinkAction::Add(routing) => {
            sub.provider.routing = Some(routing);
        }
        provider_routing::RoutingLinkAction::AddAndActivate(mut routing) => {
            routing.enabled = true;
            sub.provider.routing = Some(routing);
        }
        provider_routing::RoutingLinkAction::Off => {
            if let Some(routing) = &mut sub.provider.routing {
                routing.enabled = false;
            }
        }
    }
    let enabled = sub.provider.routing.as_ref().map(|r| r.enabled).unwrap_or(false);
    ImportOutcome::Routing {
        subscription: sub.id.clone(),
        enabled,
    }
}

/// Index of the subscription that owns the selected server.
fn subscription_of_selected(state: &AppState) -> Option<usize> {
    let id = state.selected_server.as_deref()?;
    state
        .subscriptions
        .iter()
        .position(|sub| sub.servers.iter().any(|s| s.id == id))
}

/// Human status for a share-link import.
fn links_status(added: usize, skipped: usize) -> String {
    let mut text = format!("Добавлено серверов: {added}");
    if skipped > 0 {
        text.push_str(&format!(", пропущено: {skipped}"));
    }
    text
}

/// Should this subscription be refreshed now? (`update_always` or its interval elapsed.)
fn should_update(sub: &Subscription, now: i64) -> bool {
    if sub.provider.update_always {
        return true;
    }
    if sub.update_interval_hours == 0 {
        return false;
    }
    match sub.last_update {
        None => true,
        Some(last) => now.saturating_sub(last) >= i64::from(sub.update_interval_hours) * 3600,
    }
}

/// Random hex string of `bytes` bytes (16 chars for 8 bytes).
fn random_hex(bytes: usize) -> String {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};
    use std::time::{SystemTime, UNIX_EPOCH};

    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let mut seed = RandomState::new().build_hasher();
    seed.write_u128(nanos);
    seed.write_u32(std::process::id());

    let mut out = String::with_capacity(bytes * 2);
    let mut value = seed.finish();
    while out.len() < bytes * 2 {
        for byte in value.to_le_bytes() {
            out.push_str(&format!("{byte:02x}"));
            if out.len() >= bytes * 2 {
                break;
            }
        }
        seed.write_u64(value);
        value = seed.finish();
    }
    out.truncate(bytes * 2);
    out
}

/// Bridges the Slint UI and the Flamix core.
pub struct Controller {
    pub(crate) state: Arc<Mutex<AppState>>,
    pub(crate) data_dir: PathBuf,
    pub(crate) rt: tokio::runtime::Handle,
    pub(crate) ui: Weak<AppWindow>,
    pub(crate) conn: Arc<Mutex<Option<Connection>>>,
    pub(crate) persist: crate::persist::Persist,
    pub(crate) sort_by_ping: AtomicBool,
    /// Ids of subscriptions with an update in flight (drives the spinner in the UI).
    pub(crate) updating: Arc<Mutex<HashSet<String>>>,
    /// Bounds the number of concurrent subscription fetches.
    pub(crate) updates: Arc<tokio::sync::Semaphore>,
}

impl Controller {
    /// Create the controller, load the persisted state and wire every UI callback.
    pub fn new(
        ui: &AppWindow,
        data_dir: PathBuf,
        rt: tokio::runtime::Handle,
        state: AppState,
        startup_import: Option<String>,
    ) -> Arc<Self> {
        let persist = crate::persist::Persist::spawn(data_dir.clone(), &rt);
        let controller = Arc::new(Self {
            state: Arc::new(Mutex::new(state)),
            data_dir,
            rt,
            ui: ui.as_weak(),
            conn: Arc::new(Mutex::new(None)),
            persist,
            sort_by_ping: AtomicBool::new(false),
            updating: Arc::new(Mutex::new(HashSet::new())),
            updates: Arc::new(tokio::sync::Semaphore::new(MAX_PARALLEL_UPDATES)),
        });
        controller.wire(ui);
        controller.apply_now();
        if let Some(text) = startup_import {
            controller.import_text(&text);
        }
        let auto = Arc::clone(&controller);
        controller.rt.spawn(async move { auto.run_auto_update().await });
        controller
    }

    /// Snapshot the state and paint the whole window. Must run on the UI thread.
    pub(crate) fn apply_to(&self, ui: &AppWindow) {
        let (subs, locals, name, iso, tun) = {
            let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            let updating = self.updating.lock().unwrap_or_else(|e| e.into_inner());
            let query = ui.get_search_text().to_string();
            let sort = self.sort_by_ping.load(Ordering::Relaxed);
            (
                view::build_subscriptions(&state, &query, sort, &updating),
                view::build_local_servers(&state, &query, sort),
                view::selected_name(&state),
                view::selected_iso(&state),
                state.settings.mode == ConnectionMode::Tun,
            )
        };
        ui.set_subscriptions(view::model(subs));
        ui.set_local_servers(view::model(locals));
        ui.set_selected_name(name.into());
        if iso.is_empty() {
            ui.set_selected_flag(slint::Image::default());
        } else {
            ui.set_selected_flag(flags::flag(&iso));
        }
        ui.set_tun_mode(tun);
    }

    /// Paint synchronously (used during construction, before the event loop runs).
    pub(crate) fn apply_now(&self) {
        if let Some(ui) = self.ui.upgrade() {
            self.apply_to(&ui);
        }
    }

    /// Ask the event loop to repaint; safe to call from any thread.
    pub(crate) fn request_render(self: &Arc<Self>) {
        let this = Arc::clone(self);
        let _ = self.ui.upgrade_in_event_loop(move |ui| this.apply_to(&ui));
    }

    /// Persist the current state (debounced) and repaint.
    pub(crate) fn persist_and_render(self: &Arc<Self>) {
        let snapshot = {
            let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            state.clone()
        };
        self.persist.schedule(snapshot);
        self.request_render();
    }

    /// Update the status line from any thread.
    fn set_status(&self, text: impl Into<String>) {
        let text = text.into();
        let _ = self
            .ui
            .upgrade_in_event_loop(move |ui| ui.set_status_text(text.into()));
    }

    /// Parse and apply a deep link / pasted text on the calling thread, then repaint.
    fn import_text(self: &Arc<Self>, text: &str) {
        let Some(import) = deeplink::parse_import(text) else {
            self.set_status("Не удалось распознать ссылку");
            return;
        };
        let outcome = {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            apply_import(&mut state, import)
        };
        match outcome {
            ImportOutcome::SubscriptionAdded { id, name } => {
                let label = if name.is_empty() { id.clone() } else { name };
                self.set_status(format!("Подписка добавлена: {label}"));
                self.persist_and_render();
                self.spawn_update(id);
                return;
            }
            ImportOutcome::LinksAdded { added, skipped } => {
                self.set_status(links_status(added, skipped));
            }
            ImportOutcome::Routing { enabled, .. } => {
                self.set_status(if enabled {
                    "Маршрутизация включена"
                } else {
                    "Маршрутизация отключена"
                });
            }
            ImportOutcome::Unsupported => self.set_status("нет подписки"),
        }
        self.persist_and_render();
    }

    /// Read the clipboard on a blocking thread and import its content.
    fn import_clipboard(self: &Arc<Self>) {
        let this = Arc::clone(self);
        self.rt.spawn_blocking(move || {
            match arboard::Clipboard::new().and_then(|mut clipboard| clipboard.get_text()) {
                Ok(text) => this.import_text(&text),
                Err(_) => this.set_status("Буфер обмена пуст"),
            }
        });
    }

    /// Queue a fetch for one subscription (bounded by the global semaphore).
    fn spawn_update(self: &Arc<Self>, id: String) {
        let this = Arc::clone(self);
        self.rt.spawn(async move { this.update_one(id).await });
    }

    /// Queue a fetch for every subscription.
    fn update_all(self: &Arc<Self>) {
        let ids: Vec<String> = {
            let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            state.subscriptions.iter().map(|s| s.id.clone()).collect()
        };
        for id in ids {
            self.spawn_update(id);
        }
    }

    async fn update_one(self: Arc<Self>, id: String) {
        let _permit = match self.updates.acquire().await {
            Ok(permit) => permit,
            Err(_) => return,
        };
        let (mut sub, hwid, socks_port) = {
            let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            let Some(sub) = state
                .subscriptions
                .iter()
                .find(|s| s.id == id)
                .cloned()
            else {
                return;
            };
            (sub, state.device_id.clone(), state.settings.socks_port)
        };
        let connected = self
            .conn
            .lock()
            .map(|c| c.is_some())
            .unwrap_or(false);
        let opts = FetchOptions {
            proxy: connected.then(|| format!("socks5h://127.0.0.1:{socks_port}")),
            hwid,
            timeout: None,
        };

        self.mark_updating(&id, true);
        let name = sub.name.clone();
        let result = subscription::update(&mut sub, &opts).await;
        {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(existing) = state.subscriptions.iter_mut().find(|s| s.id == id) {
                // Preserve UI-only flags a concurrent toggle may have changed.
                sub.pinned = existing.pinned;
                sub.collapsed = existing.collapsed;
                *existing = sub;
            }
        }
        self.mark_updating(&id, false);
        self.persist_and_render();
        match result {
            Ok(()) => self.set_status(format!("Подписка обновлена: {name}")),
            Err(err) => self.set_status(format!("Ошибка обновления: {err}")),
        }
    }

    fn mark_updating(self: &Arc<Self>, id: &str, on: bool) {
        {
            let mut updating = self.updating.lock().unwrap_or_else(|e| e.into_inner());
            if on {
                updating.insert(id.to_string());
            } else {
                updating.remove(id);
            }
        }
        self.request_render();
    }

    fn delete_subscription(self: &Arc<Self>, id: &str) {
        let removed = {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            let before = state.subscriptions.len();
            state.subscriptions.retain(|s| s.id != id);
            if state.selected_server.as_deref() == Some(id) {
                state.selected_server = None;
            }
            before != state.subscriptions.len()
        };
        if removed {
            self.set_status("Подписка удалена");
            self.persist_and_render();
        }
    }

    fn delete_server(self: &Arc<Self>, id: &str) {
        let removed = {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            let before = state.local_servers.len();
            state.local_servers.retain(|s| s.id != id);
            if state.selected_server.as_deref() == Some(id) {
                state.selected_server = None;
            }
            if before == state.local_servers.len() {
                let in_subscription = state.servers().any(|s| s.id == id);
                if in_subscription {
                    self.set_status("Скрытие серверов подписки не поддерживается");
                    return;
                }
            }
            before != state.local_servers.len()
        };
        if removed {
            self.persist_and_render();
        }
    }

    /// Startup fetch for out-of-date subscriptions, then a check every ten minutes.
    async fn run_auto_update(self: Arc<Self>) {
        self.auto_update_tick(true).await;
        loop {
            tokio::time::sleep(AUTO_UPDATE_INTERVAL).await;
            self.auto_update_tick(false).await;
        }
    }

    async fn auto_update_tick(self: &Arc<Self>, startup: bool) {
        let ids: Vec<String> = {
            let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            if !startup && !state.settings.auto_update_subscriptions {
                return;
            }
            let now = chrono::Utc::now().timestamp();
            state
                .subscriptions
                .iter()
                .filter(|s| should_update(s, now))
                .map(|s| s.id.clone())
                .collect()
        };
        for id in ids {
            self.spawn_update(id);
        }
    }

    fn wire(self: &Arc<Self>, ui: &AppWindow) {
        {
            let this = Arc::clone(self);
            ui.on_select_server(move |id| this.select_server(&id));
        }
        {
            let this = Arc::clone(self);
            ui.on_toggle_pin(move |id| this.toggle_pin(&id));
        }
        {
            let this = Arc::clone(self);
            ui.on_toggle_collapse(move |id| this.toggle_collapse(&id));
        }
        {
            let this = Arc::clone(self);
            ui.on_search_changed(move |_| this.request_render());
        }
        {
            let this = Arc::clone(self);
            ui.on_import_clipboard(move || this.import_clipboard());
        }
        {
            let this = Arc::clone(self);
            ui.on_update_subscription(move |id| this.spawn_update(id.to_string()));
        }
        {
            let this = Arc::clone(self);
            ui.on_update_all(move || this.update_all());
        }
        {
            let this = Arc::clone(self);
            ui.on_delete_subscription(move |id| this.delete_subscription(&id));
        }
        {
            let this = Arc::clone(self);
            ui.on_delete_server(move |id| this.delete_server(&id));
        }
        {
            let this = Arc::clone(self);
            ui.on_sort_by_ping(move || {
                let next = !this.sort_by_ping.load(Ordering::Relaxed);
                this.sort_by_ping.store(next, Ordering::Relaxed);
                this.request_render();
            });
        }
        {
            let this = Arc::clone(self);
            ui.on_toggle_edit(move || {
                if let Some(ui) = this.ui.upgrade() {
                    ui.set_edit_mode(!ui.get_edit_mode());
                }
            });
        }
        {
            let this = Arc::clone(self);
            ui.on_add_subscription(move || {
                if let Some(ui) = this.ui.upgrade() {
                    ui.set_page(1);
                }
            });
        }
        ui.set_state(ConnState::Disconnected);
        ui.set_status_text("Не подключено".into());
        ui.set_speed_text("↑ 0 KB/s   ↓ 0 KB/s".into());
    }

    fn select_server(self: &Arc<Self>, id: &str) {
        let mut changed = false;
        {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            if state.server(id).is_some() {
                state.selected_server = Some(id.to_string());
                changed = true;
            }
        }
        if changed {
            self.persist_and_render();
        }
    }

    fn toggle_pin(self: &Arc<Self>, id: &str) {
        {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(sub) = state
                .subscriptions
                .iter_mut()
                .find(|s| s.id == id)
            {
                sub.pinned = !sub.pinned;
            } else {
                return;
            }
        }
        self.persist_and_render();
    }

    fn toggle_collapse(self: &Arc<Self>, id: &str) {
        {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(sub) = state
                .subscriptions
                .iter_mut()
                .find(|s| s.id == id)
            {
                sub.collapsed = !sub.collapsed;
            } else {
                return;
            }
        }
        self.persist_and_render();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flamix_core::model::{Credentials, ProviderRouting, Security, Server, Subscription};

    fn server(id: &str) -> Server {
        Server {
            id: id.to_string(),
            name: format!("Server {id}"),
            country: Some("DE".to_string()),
            address: "example.com".to_string(),
            port: 443,
            credentials: Credentials::Trojan {
                password: "p".to_string(),
            },
            transport: Default::default(),
            security: Security::None,
            raw_link: None,
            traffic: None,
            local_traffic: Default::default(),
            last_ping_ms: None,
        }
    }

    fn subscription(id: &str) -> Subscription {
        Subscription {
            id: id.to_string(),
            name: id.to_string(),
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
            servers: vec![server(&format!("{id}-srv"))],
            provider: Default::default(),
            options: Default::default(),
        }
    }

    #[test]
    fn import_subscription_adds_it_with_random_id() {
        let mut state = AppState::default();
        let outcome = apply_import(
            &mut state,
            deeplink::Import::Subscription {
                url: "https://example.com/sub".to_string(),
                name: Some("My Sub".to_string()),
            },
        );
        match outcome {
            ImportOutcome::SubscriptionAdded { id, name } => {
                assert_eq!(name, "My Sub");
                assert_eq!(id.len(), 16);
                assert!(id.chars().all(|c| c.is_ascii_hexdigit()));
            }
            other => panic!("unexpected {other:?}"),
        }
        assert_eq!(state.subscriptions.len(), 1);
        assert_eq!(state.subscriptions[0].name, "My Sub");
        assert_eq!(state.subscriptions[0].url, "https://example.com/sub");
    }

    #[test]
    fn import_links_dedupes_by_id() {
        let link = "trojan://pw@example.com:443#A\nss://method:pw@example.com:8388#B";
        let mut state = AppState::default();
        let first = apply_import(&mut state, deeplink::Import::Links(link.to_string()));
        assert_eq!(
            first,
            ImportOutcome::LinksAdded {
                added: 2,
                skipped: 0
            }
        );
        let second = apply_import(&mut state, deeplink::Import::Links(link.to_string()));
        assert_eq!(
            second,
            ImportOutcome::LinksAdded {
                added: 0,
                skipped: 0
            }
        );
        assert_eq!(state.local_servers.len(), 2);
    }

    #[test]
    fn routing_add_and_off_targets_selected_subscription() {
        let mut state = AppState {
            subscriptions: vec![subscription("sub-a"), subscription("sub-b")],
            selected_server: Some("sub-b-srv".to_string()),
            ..AppState::default()
        };
        let routing = ProviderRouting {
            name: "P".to_string(),
            enabled: true,
            ..ProviderRouting::default()
        };
        let outcome = apply_import(
            &mut state,
            deeplink::Import::Routing(provider_routing::RoutingLinkAction::Add(routing)),
        );
        assert_eq!(
            outcome,
            ImportOutcome::Routing {
                subscription: "sub-b".to_string(),
                enabled: true
            }
        );
        assert!(state.subscriptions[0].provider.routing.is_none());
        assert_eq!(
            state.subscriptions[1].provider.routing.as_ref().unwrap().name,
            "P"
        );

        let outcome = apply_import(
            &mut state,
            deeplink::Import::Routing(provider_routing::RoutingLinkAction::Off),
        );
        assert_eq!(
            outcome,
            ImportOutcome::Routing {
                subscription: "sub-b".to_string(),
                enabled: false
            }
        );
        assert!(!state.subscriptions[1]
            .provider
            .routing
            .as_ref()
            .unwrap()
            .enabled);
    }

    #[test]
    fn routing_without_selected_subscription_is_unsupported() {
        let mut state = AppState::default();
        let outcome = apply_import(
            &mut state,
            deeplink::Import::Routing(provider_routing::RoutingLinkAction::Off),
        );
        assert_eq!(outcome, ImportOutcome::Unsupported);
    }

    #[test]
    fn on_add_activate_enables_routing() {
        let mut state = AppState {
            subscriptions: vec![subscription("sub-a")],
            selected_server: Some("sub-a-srv".to_string()),
            ..AppState::default()
        };
        let routing = ProviderRouting {
            name: "Q".to_string(),
            enabled: false,
            ..ProviderRouting::default()
        };
        apply_import(
            &mut state,
            deeplink::Import::Routing(provider_routing::RoutingLinkAction::AddAndActivate(routing)),
        );
        assert!(state.subscriptions[0]
            .provider
            .routing
            .as_ref()
            .unwrap()
            .enabled);
    }

    #[test]
    fn random_hex_has_requested_length_and_alphabet() {
        let id = random_hex(8);
        assert_eq!(id.len(), 16);
        assert!(id.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn should_update_follows_interval_and_update_always() {
        let mut sub = subscription("s");
        let now = 1_000_000i64;
        // Manual, never updated.
        assert!(!should_update(&sub, now));

        sub.update_interval_hours = 6;
        assert!(should_update(&sub, now));

        sub.last_update = Some(now - 3600);
        assert!(!should_update(&sub, now));

        sub.last_update = Some(now - 6 * 3600);
        assert!(should_update(&sub, now));

        sub.update_interval_hours = 0;
        sub.provider.update_always = true;
        assert!(should_update(&sub, now));
    }
}

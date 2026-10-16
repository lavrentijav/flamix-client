//! UI controller: owns [`AppState`] and bridges the Slint callbacks to the core/sys crates.
//!
//! The UI thread never does I/O. Every callback either mutates the in-memory state and
//! persists it, or hands work to the multi-thread tokio runtime; background results are pushed
//! back with `Weak::upgrade_in_event_loop`.

use crate::instance;
use crate::view;
use crate::{flags, AppWindow, ConnState};
use flamix_core::config::{self, CoreKind};
use flamix_core::model::ExitIp;
use flamix_core::model::{AppState, ConnectionMode, Server, Subscription};
use flamix_core::subscription::{self, FetchOptions};
use flamix_core::{deeplink, link, ping, provider_routing};
use flamix_sys::sysproxy::ProxySpec;
use flamix_sys::{autostart, elevation, runner, stats, sysproxy};
use slint::{ComponentHandle, Weak};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Maximum number of subscription fetches running at once.
const MAX_PARALLEL_UPDATES: usize = 3;

/// Interval between the periodic auto-update checks.
const AUTO_UPDATE_INTERVAL: Duration = Duration::from_secs(10 * 60);

/// Interval between traffic samples while connected.
const TRAFFIC_TICK: Duration = Duration::from_secs(1);

/// How often the exit IP of the connected server is re-measured.
const EXIT_PROBE_INTERVAL: Duration = Duration::from_secs(10 * 60);

/// Per-request timeout while probing an exit IP through a core.
const EXIT_PROBE_TIMEOUT: Duration = Duration::from_secs(8);

/// Human status while a batch exit-IP probe runs: `"Проверка IP: 3/12 …"`.
fn probe_progress_text(done: usize, total: usize) -> String {
    format!("Проверка IP: {done}/{total} …")
}

/// Resolve the requested server ids to probe: keep order, drop unknown ids and duplicates.
fn probe_target_ids(state: &AppState, ids: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for id in ids {
        if out.iter().any(|seen| seen == id) {
            continue;
        }
        if state.server(id).is_some() {
            out.push(id.clone());
        }
    }
    out
}

/// True if the live connection is the one an in-flight exit probe belongs to.
///
/// The epoch alone already distinguishes connect attempts, but the socks port is checked too:
/// a reconnect that reused the same inbound port must not let a late probe of the old core
/// write its measurement under the new server.
fn probe_store_is_current(
    conn_epoch: u64,
    epoch: u64,
    conn_server_id: &str,
    server_id: &str,
    conn_socks_port: u16,
    socks_port: u16,
) -> bool {
    ticker_still_owns(conn_epoch, epoch, conn_server_id, server_id) && conn_socks_port == socks_port
}

/// Number of contiguous localhost ports a probe core needs: the mixed inbound (`socks`), the
/// HTTP inbound (`socks + 1`) and the core stats API (`socks + 11`, the larger offset).
const PROBE_PORT_SPAN: u16 = 12;

/// Try to reserve a contiguous block of `span` localhost ports and return its base.
///
/// All ports are bound at once (and released right after) so the block is known to be free;
/// binding port 0 lets the OS pick a base. `None` if no such block could be found.
async fn reserve_port_block(span: u16) -> Option<u16> {
    for _ in 0..50 {
        let first = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.ok()?;
        let base = first.local_addr().ok()?.port();
        if base.checked_add(span - 1).is_none() {
            continue;
        }
        let mut listeners = vec![first];
        let mut complete = true;
        for offset in 1..span {
            match tokio::net::TcpListener::bind(("127.0.0.1", base + offset)).await {
                Ok(listener) => listeners.push(listener),
                Err(_) => {
                    complete = false;
                    break;
                }
            }
        }
        drop(listeners);
        if complete {
            return Some(base);
        }
    }
    None
}

/// Clears the `probing` flag when dropped, so a panic/abort cannot leave it stuck `true`.
struct ProbeGuard(Arc<AtomicBool>);

impl Drop for ProbeGuard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

/// Persist accumulated traffic every N ticks (avoid a disk write every second).
const TRAFFIC_PERSIST_EVERY: u32 = 15;

/// Everything needed to talk to the running cores while connected.
pub struct Connection {
    running: runner::Running,
    server_id: String,
    /// Monotonic id of the connect that created this `Connection`. A ticker belongs to exactly
    /// one connect; a same-id reconnect (now common on settings/failover edits) bumps the
    /// epoch so the previous ticker stops sampling the new `Connection`.
    epoch: u64,
    /// Core that owns the outbound to the server (its traffic counters are sampled).
    core: CoreKind,
    api_port: u16,
    /// Local SOCKS inbound port of this connection (used to attribute exit-IP probes).
    socks_port: u16,
    /// Xray binary, needed for `xray api statsquery`.
    xray_bin: Option<PathBuf>,
    /// Whether the running config is a failover cluster: Xray counters then live on the
    /// `node-N` outbounds, not the single `proxy` outbound.
    failover: bool,
    /// Whether we changed the OS system proxy (so we must clear it on exit).
    system_proxy: bool,
    /// Last cumulative totals reported by sing-box (deltas are computed from these).
    totals: (u64, u64),
}

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

fn apply_routing(
    state: &mut AppState,
    action: provider_routing::RoutingLinkAction,
) -> ImportOutcome {
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
    let enabled = sub
        .provider
        .routing
        .as_ref()
        .map(|r| r.enabled)
        .unwrap_or(false);
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

/// Build the `[primary, backups…]` node slice for core-level failover.
///
/// `primary_id` is the selected server; `settings.failover.backups` are resolved through
/// [`AppState::server`] in order. Unknown ids and the primary's own id are skipped (dedupe),
/// keeping the user's order for the rest. Returns an empty slice when the primary is unknown.
/// Pure, so the selection logic is unit-tested without a live core.
fn failover_nodes(state: &AppState, primary_id: &str) -> Vec<Server> {
    let Some(primary) = state.server(primary_id).cloned() else {
        return Vec::new();
    };
    let mut nodes = vec![primary];
    let mut seen: HashSet<String> = HashSet::new();
    seen.insert(primary_id.to_string());
    for id in &state.settings.failover.backups {
        if !seen.insert(id.clone()) {
            continue;
        }
        if let Some(backup) = state.server(id) {
            nodes.push(backup.clone());
        }
    }
    nodes
}

/// Whether the planned config actually runs a failover cluster (outbounds tagged `node-N`).
///
/// Failover can be enabled in settings while the cluster still collapses to a single node
/// (no usable backups); the counters then stay on the single `proxy` outbound.
fn plans_use_failover(plans: &[config::CorePlan]) -> bool {
    plans.iter().any(|plan| {
        plan.config
            .get("outbounds")
            .and_then(serde_json::Value::as_array)
            .map(|outbounds| {
                outbounds.iter().any(|outbound| {
                    outbound
                        .get("tag")
                        .and_then(serde_json::Value::as_str)
                        .is_some_and(|tag| tag.starts_with("node-"))
                })
            })
            .unwrap_or(false)
    })
}

/// Whether a traffic ticker still owns the current [`Connection`].
///
/// A ticker is bound to the epoch of the connect that spawned it. When a reconnect replaces
/// the `Connection` — even to the same `server_id` — the epoch no longer matches and the old
/// ticker must stop, so two samplers never race on `conn.totals` (which would double-count
/// sing-box deltas). Pure, so the guard is unit-tested without a live core.
fn ticker_still_owns(conn_epoch: u64, epoch: u64, conn_server_id: &str, server_id: &str) -> bool {
    conn_epoch == epoch && conn_server_id == server_id
}

/// Human status for a share-link import.
fn links_status(added: usize, skipped: usize) -> String {
    let mut text = format!("Добавлено серверов: {added}");
    if skipped > 0 {
        text.push_str(&format!(", пропущено: {skipped}"));
    }
    text
}

/// Extract the import string (positional deep link or `--import <value>`) from raw args.
pub(crate) fn extract_import(args: &[String]) -> Option<String> {
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--screenshot" => {
                let _ = it.next();
            }
            "--import" => return it.next().cloned(),
            "--light" | "--demo" | "--minimized" | "--" => {}
            other if other.starts_with('-') => {}
            other => return Some(other.to_string()),
        }
    }
    None
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

/// First non-empty line of a possibly multi-line error / log tail.
fn first_line(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("")
        .to_string()
}

/// Marker written while the OS system proxy points at us (crash detection).
fn proxy_marker(dir: &Path) -> PathBuf {
    dir.join("run").join("proxy-set")
}

/// Create the proxy marker file (called after a successful `sysproxy::set`).
fn mark_proxy_set(dir: &Path) {
    let path = proxy_marker(dir);
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Err(err) = std::fs::write(&path, b"") {
        tracing::warn!(%err, "could not write the proxy marker");
    }
}

/// Remove the proxy marker file (called after `sysproxy::clear`).
fn clear_proxy_marker(dir: &Path) {
    let _ = std::fs::remove_file(proxy_marker(dir));
}

/// "YYYY-MM-DD" in the machine's local timezone.
fn today_key() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

/// Publish the system proxy (blocking work moved off the async threads).
async fn set_system_proxy(spec: ProxySpec) -> Result<(), String> {
    match tokio::task::spawn_blocking(move || sysproxy::set(&spec)).await {
        Ok(Ok(())) => Ok(()),
        Ok(Err(err)) => Err(err.to_string()),
        Err(err) => Err(err.to_string()),
    }
}

/// Clear the system proxy (blocking work moved off the async threads).
async fn clear_system_proxy() -> Result<(), String> {
    match tokio::task::spawn_blocking(sysproxy::clear).await {
        Ok(Ok(())) => Ok(()),
        Ok(Err(err)) => Err(err.to_string()),
        Err(err) => Err(err.to_string()),
    }
}

/// Random hex string of `bytes` bytes (16 chars for 8 bytes).
pub(crate) fn random_hex(bytes: usize) -> String {
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
    /// True while a connect/disconnect transition is in flight.
    pub(crate) connecting: Arc<AtomicBool>,
    /// Bumped on every `connect`, stamped onto the new [`Connection`] and captured by its
    /// ticker so only the newest ticker for a given `server_id` keeps running.
    pub(crate) connect_epoch: AtomicU64,
    /// True while a batch exit-IP probe is running (only one at a time).
    pub(crate) probing: Arc<AtomicBool>,
    /// Generation counter bumped on every disconnect; the passive exit-IP poller subscribes
    /// to it so a disconnect cancels its wait immediately instead of after the full interval.
    pub(crate) exit_probe_cancel: tokio::sync::watch::Sender<u64>,
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
            connecting: Arc::new(AtomicBool::new(false)),
            connect_epoch: AtomicU64::new(0),
            probing: Arc::new(AtomicBool::new(false)),
            exit_probe_cancel: tokio::sync::watch::channel(0).0,
        });
        controller.wire(ui);
        controller.apply_now();
        if let Some(text) = startup_import {
            controller.import_text(&text);
        }
        let auto = Arc::clone(&controller);
        controller
            .rt
            .spawn(async move { auto.run_auto_update().await });
        let cleanup = Arc::clone(&controller);
        controller
            .rt
            .spawn(async move { cleanup.cleanup_stale_proxy().await });
        let signal = Arc::clone(&controller);
        controller.rt.spawn(async move {
            if tokio::signal::ctrl_c().await.is_ok() {
                signal.initiate_shutdown();
            }
        });
        let autostart = Arc::clone(&controller);
        controller
            .rt
            .spawn(async move { autostart.sync_autostart().await });
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

    /// Show or hide the "restart as administrator" button in the connection panel.
    fn set_needs_admin(&self, value: bool) {
        let _ = self
            .ui
            .upgrade_in_event_loop(move |ui| ui.set_needs_admin(value));
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

    /// Handle arguments forwarded by a second launch: import and bring the window to front.
    pub fn handle_remote_args(self: &Arc<Self>, args: Vec<String>) {
        let _ = self.ui.upgrade_in_event_loop(|ui| {
            let _ = ui.show();
        });
        if let Some(text) = extract_import(&args) {
            self.import_text(&text);
        }
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
            let Some(sub) = state.subscriptions.iter().find(|s| s.id == id).cloned() else {
                return;
            };
            (sub, state.device_id.clone(), state.settings.socks_port)
        };
        let connected = self.conn.lock().map(|c| c.is_some()).unwrap_or(false);
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
            let selected_gone = state
                .selected_server
                .as_deref()
                .map(|selected| state.server(selected).is_none())
                .unwrap_or(false);
            if selected_gone {
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

    // --- connect / disconnect ------------------------------------------------

    /// Connected state of the whole app, mirrored into the UI.
    fn set_conn_state(&self, state: ConnState, status: impl Into<String>) {
        let status = status.into();
        let _ = self.ui.upgrade_in_event_loop(move |ui| {
            ui.set_state(state);
            ui.set_status_text(status.into());
        });
    }

    fn set_speed(&self, up: u64, down: u64) {
        let text = format!(
            "↑ {}/s   ↓ {}/s",
            view::human_bytes(up),
            view::human_bytes(down)
        );
        let _ = self
            .ui
            .upgrade_in_event_loop(move |ui| ui.set_speed_text(text.into()));
    }

    fn toggle_connect(self: &Arc<Self>) {
        if self.connecting.load(Ordering::SeqCst) {
            return;
        }
        let connected = self.conn.lock().map(|c| c.is_some()).unwrap_or(false);
        if connected {
            self.start_disconnect();
            return;
        }
        let server_id = self
            .state
            .lock()
            .map(|s| s.selected_server.clone())
            .unwrap_or(None);
        match server_id {
            Some(id) => self.start_connect(id),
            None => self.set_conn_state(ConnState::Disconnected, "Выберите сервер"),
        }
    }

    fn start_connect(self: &Arc<Self>, server_id: String) {
        if self.connecting.swap(true, Ordering::SeqCst) {
            return;
        }
        let this = Arc::clone(self);
        self.rt.spawn(async move {
            Arc::clone(&this).connect(server_id).await;
            this.connecting.store(false, Ordering::SeqCst);
        });
    }

    fn start_disconnect(self: &Arc<Self>) {
        if self.connecting.swap(true, Ordering::SeqCst) {
            return;
        }
        let this = Arc::clone(self);
        self.rt.spawn(async move {
            this.disconnect_internal().await;
            this.connecting.store(false, Ordering::SeqCst);
        });
    }

    async fn connect(self: Arc<Self>, server_id: String) {
        // A batch exit-IP probe owns `settings.socks_port` and the cores: starting a
        // connection now would race it. Refuse and let the user retry.
        if self.probing.load(Ordering::SeqCst) {
            self.set_conn_state(ConnState::Error, "Идёт проверка выходных IP, подождите");
            return;
        }
        // Selecting another server while connected reconnects: tear the old one down first.
        self.disconnect_internal().await;

        let (server, settings, provider, overrides, failover_on, nodes) = {
            let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            let Some(server) = state.server(&server_id).cloned() else {
                drop(state);
                self.set_conn_state(ConnState::Disconnected, "Выберите сервер");
                return;
            };
            // Provider routing/overrides come from the subscription that owns the server.
            // Local servers, and subscriptions with `ignore_provider_routing`, use defaults.
            let sub = state
                .subscriptions
                .iter()
                .find(|s| s.servers.iter().any(|srv| srv.id == server_id));
            let (provider, overrides) = match sub {
                Some(sub) if !sub.options.ignore_provider_routing => (
                    sub.provider.routing.clone(),
                    Some(sub.provider.app_overrides.clone()),
                ),
                _ => (None, None),
            };
            // Failover is Proxy-only (T21); TUN keeps using `plan_with`.
            let failover_on =
                state.settings.failover.enabled && state.settings.mode == ConnectionMode::Proxy;
            let nodes = if failover_on {
                failover_nodes(&state, &server_id)
            } else {
                Vec::new()
            };
            (
                server,
                state.settings.clone(),
                provider,
                overrides,
                failover_on,
                nodes,
            )
        };
        let ctx = config::ConnectContext {
            provider: provider.as_ref(),
            overrides: overrides.as_ref(),
        };

        if settings.mode == ConnectionMode::Tun && cfg!(windows) && !elevation::is_elevated() {
            self.set_needs_admin(true);
            self.set_conn_state(ConnState::Error, "Для TUN нужен запуск от администратора");
            return;
        }
        self.set_needs_admin(false);

        self.set_conn_state(ConnState::Connecting, "Подключение…");
        let plans = if failover_on && nodes.len() > 1 {
            config::plan_cluster(&nodes, &settings, &ctx)
        } else {
            config::plan_with(&server, &settings, &ctx)
        };
        // Only a real cluster moves the counters to the `node-N` outbounds.
        let failover = plans_use_failover(&plans);
        let run_dir = self.data_dir.join("run");
        let running = match runner::start(&plans, &run_dir).await {
            Ok(running) => running,
            Err(err) => {
                self.set_conn_state(
                    ConnState::Error,
                    format!("Ошибка: {}", first_line(&err.to_string())),
                );
                return;
            }
        };

        // The core with the outbound to the server owns the traffic counters.
        let owner = plans
            .iter()
            .find(|p| p.core == CoreKind::Xray)
            .or_else(|| plans.first());
        let (core, api_port) = match owner {
            Some(plan) => (plan.core, plan.api_port),
            None => (CoreKind::Xray, 0),
        };
        let xray_bin = (core == CoreKind::Xray)
            .then(|| runner::find_core(CoreKind::Xray).ok())
            .flatten();

        // Local inbound auth breaks the OS system proxy (browsers do not send proxy
        // credentials): keep working for HTTP clients only when auth is off.
        let proxy_requested = settings.mode == ConnectionMode::Proxy && settings.set_system_proxy;
        let auth_on = config::local_auth_enabled(overrides.as_ref());
        let system_proxy = proxy_requested && !auth_on;
        if system_proxy {
            let spec = ProxySpec {
                host: "127.0.0.1".to_string(),
                http_port: settings.http_port,
                socks_port: settings.socks_port,
                bypass: settings.proxy_bypass.clone(),
            };
            // Marker first: if we die between set and the marker, the next start must still clear it.
            mark_proxy_set(&self.data_dir);
            if let Err(err) = set_system_proxy(spec).await {
                clear_proxy_marker(&self.data_dir);
                let _ = running.stop().await;
                self.set_conn_state(
                    ConnState::Error,
                    format!("Ошибка системного прокси: {}", first_line(&err)),
                );
                return;
            }
        }

        let epoch = self.connect_epoch.fetch_add(1, Ordering::SeqCst) + 1;
        *self.conn.lock().unwrap_or_else(|e| e.into_inner()) = Some(Connection {
            running,
            server_id: server_id.clone(),
            epoch,
            core,
            api_port,
            socks_port: settings.socks_port,
            xray_bin,
            failover,
            system_proxy,
            totals: (0, 0),
        });
        if proxy_requested && auth_on {
            self.set_conn_state(
                ConnState::Connected,
                "Системный прокси выключен: провайдер включил авторизацию",
            );
        } else {
            self.set_conn_state(ConnState::Connected, "Подключено");
        }
        self.set_speed(0, 0);
        self.start_ticker(server_id.clone(), epoch);
        self.start_exit_probe(server_id, epoch, settings.socks_port);
    }

    /// Spawn the passive exit-IP poller for a freshly connected server.
    fn start_exit_probe(self: &Arc<Self>, server_id: String, epoch: u64, socks_port: u16) {
        let this = Arc::clone(self);
        self.rt.spawn(async move {
            this.run_exit_probe(server_id, epoch, socks_port).await;
        });
    }

    /// Measure the exit IP through the running proxy now, then every [`EXIT_PROBE_INTERVAL`].
    ///
    /// Uses the same epoch/server/port guard as the traffic ticker, and subscribes to
    /// [`Controller::exit_probe_cancel`] so a disconnect (or reconnect) aborts the wait at
    /// once instead of after a full interval. The guard is re-checked after the (awaited)
    /// probe and again under the conn lock right before storing, so a late response cannot be
    /// attributed to a newer connection that happens to reuse the same port.
    async fn run_exit_probe(self: Arc<Self>, server_id: String, epoch: u64, socks_port: u16) {
        let proxy = format!("socks5h://127.0.0.1:{socks_port}");
        let mut cancel = self.exit_probe_cancel.subscribe();
        loop {
            if !self.conn_matches(&server_id, epoch, socks_port) {
                return;
            }
            if let Some(ip) = ping::probe_exit_ip(&proxy, EXIT_PROBE_TIMEOUT).await {
                if !self.store_exit_ip_if_current(&server_id, epoch, socks_port, ip) {
                    return;
                }
            } else if !self.conn_matches(&server_id, epoch, socks_port) {
                return;
            }
            tokio::select! {
                _ = tokio::time::sleep(EXIT_PROBE_INTERVAL) => {}
                changed = cancel.changed() => {
                    if changed.is_err() {
                        return;
                    }
                    // A newer generation means disconnect/reconnect: stop quietly.
                    return;
                }
            }
        }
    }

    /// Whether the current live connection is still the one identified by `epoch`/`server_id`
    /// and uses `socks_port`.
    fn conn_matches(&self, server_id: &str, epoch: u64, socks_port: u16) -> bool {
        self.conn
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .map(|c| {
                probe_store_is_current(
                    c.epoch,
                    epoch,
                    &c.server_id,
                    server_id,
                    c.socks_port,
                    socks_port,
                )
            })
            .unwrap_or(false)
    }

    /// Store `ip` for `server_id` only if the live connection is still the probe's own
    /// connection (epoch, server id and socks port checked under the conn lock).
    fn store_exit_ip_if_current(
        self: &Arc<Self>,
        server_id: &str,
        epoch: u64,
        socks_port: u16,
        ip: String,
    ) -> bool {
        {
            let guard = self.conn.lock().unwrap_or_else(|e| e.into_inner());
            match guard.as_ref() {
                Some(conn)
                    if probe_store_is_current(
                        conn.epoch,
                        epoch,
                        &conn.server_id,
                        server_id,
                        conn.socks_port,
                        socks_port,
                    ) => {}
                _ => return false,
            }
        }
        self.store_exit_ip(server_id, ip);
        true
    }

    /// Persist a measured exit IP for one server and repaint.
    fn store_exit_ip(self: &Arc<Self>, id: &str, ip: String) {
        let checked = chrono::Utc::now().timestamp();
        self.mutate(|state| {
            state
                .exit_ips
                .insert(id.to_string(), ExitIp { ip, checked });
        });
    }

    /// Probe the exit IP of every requested server by really connecting through it.
    ///
    /// Refuses while a core is busy (connecting, connected, or another batch probing — they
    /// would share `run/` config/pid/stop files and the inbound). Sequentially, for each id:
    /// reserve a fresh contiguous port block, plan a throwaway proxy config, start the core in
    /// a dedicated work dir, ask the echo services through its SOCKS port, and stop the core
    /// in every path (result or failure) so none leaks.
    pub fn probe_exit_ips(self: &Arc<Self>, ids: Vec<String>) {
        if self.connecting.load(Ordering::SeqCst) || self.is_connected() {
            self.status("Сначала отключитесь");
            return;
        }
        if self.probing.swap(true, Ordering::SeqCst) {
            self.status("Проверка уже выполняется");
            return;
        }
        let guard = ProbeGuard(Arc::clone(&self.probing));
        let snapshot = self.state.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let targets = probe_target_ids(&snapshot, &ids);
        let runner = Arc::clone(self);
        self.rt.spawn(async move {
            // Keep the guard alive for the whole task; it clears `probing` on drop, even if
            // the task panics or is aborted.
            let _guard = guard;
            runner.run_exit_probe_batch(snapshot, targets).await;
        });
    }

    async fn run_exit_probe_batch(self: Arc<Self>, state: AppState, targets: Vec<String>) {
        let total = targets.len();
        if total == 0 {
            self.set_status("Нет серверов для проверки IP");
            return;
        }
        let mut measured = 0usize;
        for (index, id) in targets.iter().enumerate() {
            self.set_status(probe_progress_text(index, total));
            if let Some(ip) = self.probe_one_exit(&state, id).await {
                measured += 1;
                self.store_exit_ip(id, ip);
            }
        }
        self.persist_and_render();
        if measured == 0 {
            self.set_status("Не удалось определить выходной IP");
        } else {
            self.set_status(format!(
                "Готово: измерено IP у {measured} из {total} серверов"
            ));
        }
    }

    /// Start one throwaway core for `id` in the probe's private work dir and read its egress
    /// IP. Retries once with a fresh port block in case the reserved ports were grabbed
    /// between reservation and `runner::start` (a small TOCTOU window).
    async fn probe_one_exit(self: &Arc<Self>, state: &AppState, id: &str) -> Option<String> {
        let run_dir = self.data_dir.join("run").join("exit-probe");
        for _attempt in 0..2 {
            let base = reserve_port_block(PROBE_PORT_SPAN).await?;
            let server = state.server(id).cloned()?;
            let mut settings = state.settings.clone();
            settings.mode = ConnectionMode::Proxy;
            settings.socks_port = base;
            settings.http_port = base.saturating_add(1);
            let plans = config::plan(&server, &settings);
            match runner::start(&plans, &run_dir).await {
                Ok(running) => {
                    let proxy = format!("socks5h://127.0.0.1:{base}");
                    let ip = ping::probe_exit_ip(&proxy, EXIT_PROBE_TIMEOUT).await;
                    if let Err(err) = running.stop().await {
                        tracing::warn!(%err, server = %id, "could not stop the probe core cleanly");
                    }
                    return ip;
                }
                Err(err) => {
                    tracing::warn!(%err, server = %id, "exit-IP probe could not start the core; retrying");
                }
            }
        }
        None
    }

    async fn disconnect_internal(self: &Arc<Self>) {
        // Cancel any passive exit probe immediately (even if there is no connection yet).
        self.exit_probe_cancel
            .send_modify(|generation| *generation = generation.wrapping_add(1));
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner()).take();
        let Some(conn) = conn else {
            return;
        };
        if conn.system_proxy {
            if let Err(err) = clear_system_proxy().await {
                tracing::warn!(%err, "could not clear the system proxy");
            }
        }
        clear_proxy_marker(&self.data_dir);
        if let Err(err) = conn.running.stop().await {
            tracing::warn!(%err, "could not stop the core cleanly");
        }
        self.set_conn_state(ConnState::Disconnected, "Не подключено");
        self.set_speed(0, 0);
        self.persist_and_render();
    }

    fn start_ticker(self: &Arc<Self>, server_id: String, epoch: u64) {
        let this = Arc::clone(self);
        self.rt
            .spawn(async move { this.run_ticker(server_id, epoch).await });
    }

    async fn run_ticker(self: Arc<Self>, server_id: String, epoch: u64) {
        let mut ticks = 0u32;
        loop {
            tokio::time::sleep(TRAFFIC_TICK).await;
            let alive = {
                let mut guard = self.conn.lock().unwrap_or_else(|e| e.into_inner());
                match guard.as_mut() {
                    Some(conn)
                        if ticker_still_owns(conn.epoch, epoch, &conn.server_id, &server_id) =>
                    {
                        conn.running.is_alive()
                    }
                    // A newer connect replaced this one (even to the same id): stop.
                    _ => return,
                }
            };
            if !alive {
                self.on_core_dead(&server_id, epoch).await;
                return;
            }
            let (up, down) = self.sample_traffic(&server_id, epoch).await;
            self.set_speed(up, down);
            ticks += 1;
            if ticks >= TRAFFIC_PERSIST_EVERY {
                ticks = 0;
                self.persist_and_render();
            }
        }
    }

    /// Read the traffic delta of the owning core and accumulate it into the state.
    async fn sample_traffic(self: &Arc<Self>, server_id: &str, epoch: u64) -> (u64, u64) {
        let sample = {
            let guard = self.conn.lock().unwrap_or_else(|e| e.into_inner());
            let Some(conn) = guard.as_ref() else {
                return (0, 0);
            };
            if !ticker_still_owns(conn.epoch, epoch, &conn.server_id, server_id) {
                return (0, 0);
            }
            (
                conn.core,
                conn.api_port,
                conn.xray_bin.clone(),
                conn.totals,
                conn.failover,
            )
        };
        let (core, api_port, xray_bin, totals, failover) = sample;
        let delta = match core {
            CoreKind::Xray => {
                let Some(binary) = xray_bin else {
                    return (0, 0);
                };
                let pattern = if failover {
                    stats::XrayPattern::Cluster
                } else {
                    stats::XrayPattern::Proxy
                };
                stats::xray_traffic(&binary, api_port, pattern).await.ok()
            }
            CoreKind::SingBox => match stats::singbox_traffic(api_port).await {
                Ok((up_total, down_total)) => {
                    {
                        let mut guard = self.conn.lock().unwrap_or_else(|e| e.into_inner());
                        if let Some(conn) = guard.as_mut() {
                            if ticker_still_owns(conn.epoch, epoch, &conn.server_id, server_id) {
                                conn.totals = (up_total, down_total);
                            }
                        }
                    }
                    Some((
                        up_total.saturating_sub(totals.0),
                        down_total.saturating_sub(totals.1),
                    ))
                }
                Err(_) => None,
            },
        };
        let Some((up, down)) = delta else {
            return (0, 0);
        };
        {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(server) = state.server_mut(server_id) {
                server.local_traffic.upload = server.local_traffic.upload.saturating_add(up);
                server.local_traffic.download = server.local_traffic.download.saturating_add(down);
            }
            let daily = state.daily_traffic.entry(today_key()).or_default();
            daily.upload = daily.upload.saturating_add(up);
            daily.download = daily.download.saturating_add(down);
        }
        (up, down)
    }

    async fn on_core_dead(self: &Arc<Self>, server_id: &str, epoch: u64) {
        let conn = {
            let mut guard = self.conn.lock().unwrap_or_else(|e| e.into_inner());
            if guard
                .as_ref()
                .map(|c| ticker_still_owns(c.epoch, epoch, &c.server_id, server_id))
                .unwrap_or(false)
            {
                guard.take()
            } else {
                None
            }
        };
        let Some(conn) = conn else {
            return;
        };
        // The connection is gone: stop the passive exit probe from waiting out its interval.
        self.exit_probe_cancel
            .send_modify(|generation| *generation = generation.wrapping_add(1));
        let tail = conn.running.log_tail(5).join(" ");
        if conn.system_proxy {
            let _ = clear_system_proxy().await;
        }
        clear_proxy_marker(&self.data_dir);
        let _ = conn.running.stop().await;
        self.set_conn_state(
            ConnState::Error,
            format!("ядро остановилось: {}", first_line(&tail)),
        );
        self.set_speed(0, 0);
        self.persist_and_render();
    }

    /// Clear a system proxy left behind by a previous crash (marker file present).
    async fn cleanup_stale_proxy(self: Arc<Self>) {
        if !proxy_marker(&self.data_dir).exists() {
            return;
        }
        tracing::info!("proxy marker from a previous run found; clearing the system proxy");
        if let Err(err) = clear_system_proxy().await {
            tracing::warn!(%err, "could not clear the stale system proxy");
        }
        clear_proxy_marker(&self.data_dir);
    }

    /// Make the OS autostart entry match `settings.launch_at_login`.
    async fn sync_autostart(self: Arc<Self>) {
        let desired = {
            let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            state.settings.launch_at_login
        };
        let result = tokio::task::spawn_blocking(move || {
            if autostart::is_enabled() == desired {
                return Ok(());
            }
            autostart::set_enabled(desired)
        })
        .await;
        match result {
            Ok(Ok(())) => {}
            Ok(Err(err)) => tracing::warn!(%err, "could not update autostart"),
            Err(err) => tracing::warn!(%err, "autostart task failed"),
        }
    }

    fn mode_changed(self: &Arc<Self>, tun: bool) {
        let needs_admin = tun && cfg!(windows) && !elevation::is_elevated();
        self.set_needs_admin(needs_admin);
        if needs_admin {
            self.set_status("Для TUN нужен запуск от администратора");
        }
        let new_mode = if tun {
            ConnectionMode::Tun
        } else {
            ConnectionMode::Proxy
        };
        let changed = {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            if state.settings.mode == new_mode {
                false
            } else {
                state.settings.mode = new_mode;
                true
            }
        };
        if !changed {
            return;
        }
        self.persist_and_render();
        let connected_id = self
            .conn
            .lock()
            .ok()
            .and_then(|c| c.as_ref().map(|conn| conn.server_id.clone()));
        if let Some(id) = connected_id {
            self.start_connect(id);
        }
    }

    fn restart_as_admin(self: &Arc<Self>) {
        let args: Vec<String> = std::env::args().skip(1).collect();
        match elevation::restart_elevated(&args) {
            Ok(()) => {
                let _ = std::fs::remove_file(instance::instance_path(&self.data_dir));
                let _ = slint::quit_event_loop();
            }
            Err(err) => self.set_status(format!("Не удалось перезапустить: {err}")),
        }
    }

    // --- ping ----------------------------------------------------------------

    fn set_ping_result(&self, text: String) {
        let _ = self
            .ui
            .upgrade_in_event_loop(move |ui| ui.set_ping_result(text.into()));
    }

    /// TCP-ping every visible server (concurrency 16, 3 s timeout).
    fn ping_all(self: &Arc<Self>) {
        let query = self
            .ui
            .upgrade()
            .map(|ui| ui.get_search_text().to_string())
            .unwrap_or_default();
        let targets: Vec<(String, String, u16)> = {
            let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            view::visible_servers(&state, &query)
                .iter()
                .map(|s| (s.id.clone(), s.address.clone(), s.port))
                .collect()
        };
        if targets.is_empty() {
            self.set_status("Нет серверов для проверки");
            return;
        }
        let this = Arc::clone(self);
        self.rt.spawn(async move {
            let results = ping::tcp_ping_many(targets, Duration::from_secs(3), 16).await;
            {
                let mut state = this.state.lock().unwrap_or_else(|e| e.into_inner());
                for (id, ms) in results {
                    if let Some(server) = state.server_mut(&id) {
                        server.last_ping_ms = Some(ms);
                    }
                }
            }
            this.persist_and_render();
        });
    }

    /// Ping the selected server: URL through the proxy when connected, else TCP connect.
    fn ping_selected(self: &Arc<Self>) {
        let (id, address, port, ping_url, ping_method, socks_port) = {
            let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            let Some(id) = state.selected_server.clone() else {
                return;
            };
            let Some(server) = state.server(&id) else {
                return;
            };
            (
                id,
                server.address.clone(),
                server.port,
                state.settings.ping_url.clone(),
                state.settings.ping_method.clone(),
                state.settings.socks_port,
            )
        };
        let connected = self.conn.lock().map(|c| c.is_some()).unwrap_or(false);
        let this = Arc::clone(self);
        self.rt.spawn(async move {
            let ms = if connected && ping_method == "url" {
                let proxy = format!("socks5h://127.0.0.1:{socks_port}");
                ping::url_ping(&ping_url, &proxy, Duration::from_secs(5)).await
            } else {
                ping::tcp_ping(&address, port, Duration::from_secs(3)).await
            };
            {
                let mut state = this.state.lock().unwrap_or_else(|e| e.into_inner());
                if let Some(server) = state.server_mut(&id) {
                    server.last_ping_ms = Some(ms);
                }
            }
            this.set_ping_result(match ms {
                Some(ms) => format!(" · {ms} ms"),
                None => " · нет ответа".to_string(),
            });
            this.persist_and_render();
        });
    }

    /// Disconnect, save synchronously and quit (window close / Ctrl+C).
    fn initiate_shutdown(self: &Arc<Self>) {
        let this = Arc::clone(self);
        self.rt.spawn(async move {
            this.disconnect_internal().await;
            let snapshot = this.state.lock().unwrap_or_else(|e| e.into_inner()).clone();
            if let Err(err) = this.persist.save_now(&snapshot) {
                tracing::warn!(%err, "could not save state on exit");
            }
            let _ = std::fs::remove_file(instance::instance_path(&this.data_dir));
            let _ = slint::quit_event_loop();
        });
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
            ui.on_toggle_connect(move || this.toggle_connect());
        }
        {
            let this = Arc::clone(self);
            ui.on_mode_changed(move |tun| this.mode_changed(tun));
        }
        {
            let this = Arc::clone(self);
            ui.on_restart_as_admin(move || this.restart_as_admin());
        }
        {
            let this = Arc::clone(self);
            ui.on_ping_all(move || this.ping_all());
        }
        {
            let this = Arc::clone(self);
            ui.on_ping_selected(move || this.ping_selected());
        }
        {
            let this = Arc::clone(self);
            ui.on_check_exit_ips(move || {
                let ids: Vec<String> =
                    this.with_state(|s| s.servers().map(|server| server.id.clone()).collect());
                this.probe_exit_ips(ids);
            });
        }
        {
            let this = Arc::clone(self);
            ui.window().on_close_requested(move || {
                this.initiate_shutdown();
                slint::CloseRequestResponse::KeepWindowShown
            });
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
            if state.server(id).is_some() && state.selected_server.as_deref() != Some(id) {
                state.selected_server = Some(id.to_string());
                changed = true;
            }
        }
        if !changed {
            return;
        }
        self.set_ping_result(String::new());
        self.persist_and_render();
        // Selecting another server while connected reconnects to it.
        let connected_other = self
            .conn
            .lock()
            .ok()
            .and_then(|c| c.as_ref().map(|conn| conn.server_id != id))
            .unwrap_or(false);
        if connected_other {
            self.start_connect(id.to_string());
        }
    }

    fn toggle_pin(self: &Arc<Self>, id: &str) {
        {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(sub) = state.subscriptions.iter_mut().find(|s| s.id == id) {
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
            if let Some(sub) = state.subscriptions.iter_mut().find(|s| s.id == id) {
                sub.collapsed = !sub.collapsed;
            } else {
                return;
            }
        }
        self.persist_and_render();
    }
}

/// API for the secondary pages (`src/pages/*`). Pages must not touch private fields.
#[allow(dead_code)] // filled in by the page tasks (docs/tasks/T17-pages.md)
impl Controller {
    /// Read the state (clone what you need; never hold the lock across await/UI calls).
    pub fn with_state<R>(&self, f: impl FnOnce(&AppState) -> R) -> R {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        f(&state)
    }

    /// Mutate the state, then persist (debounced) and re-render the servers view.
    pub fn mutate(self: &Arc<Self>, f: impl FnOnce(&mut AppState)) {
        {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            f(&mut state);
        }
        self.persist_and_render();
    }

    /// Import a subscription URL / share links / deep link exactly like the clipboard import.
    pub fn import(self: &Arc<Self>, text: &str) {
        self.import_text(text);
    }

    /// True while a core is running.
    pub fn is_connected(&self) -> bool {
        self.conn
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_some()
    }

    /// Apply changed settings/routing: reconnect if connected (no-op otherwise).
    ///
    /// Any core-affecting edit must call this — the settings page already does it for its
    /// fields, and it **must also call it whenever `settings.failover` changes** (enabling or
    /// disabling failover, or editing the backup list), since the running core config embeds
    /// the primary + backups. The next [`Self::connect`] re-reads `state.settings` fresh.
    pub fn reconnect_if_connected(self: &Arc<Self>) {
        let selected = self.with_state(|s| s.selected_server.clone());
        if let (true, Some(id)) = (self.is_connected(), selected) {
            self.start_connect(id);
        }
    }

    /// Tokio runtime handle for page background work.
    pub fn runtime(&self) -> tokio::runtime::Handle {
        self.rt.clone()
    }

    /// Show a line in the status text of the connection panel.
    pub fn status(&self, text: impl Into<String>) {
        self.set_status(text);
    }
}

/// Small wrappers used by the system tray (`src/tray.rs`). They must run on the UI thread
/// or hand work to the runtime themselves, exactly like the UI callbacks do.
impl Controller {
    /// Snapshot the tray renders: `(connected, server name, tun mode)`.
    pub(crate) fn connection_state(&self) -> (bool, String, bool) {
        let connected = self.conn.lock().map(|c| c.is_some()).unwrap_or(false)
            || self.connecting.load(Ordering::Relaxed);
        let (server, tun) = {
            let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            (
                view::selected_name(&state),
                state.settings.mode == ConnectionMode::Tun,
            )
        };
        (connected, server, tun)
    }

    /// Tray "Подключиться"/"Отключиться".
    pub(crate) fn toggle_connect_public(self: &Arc<Self>) {
        self.toggle_connect();
    }

    /// Tray Proxy/TUN switch.
    pub(crate) fn set_mode_public(self: &Arc<Self>, tun: bool) {
        self.mode_changed(tun);
    }

    /// Tray "Выход": disconnect, save and stop the event loop.
    pub(crate) fn quit(self: &Arc<Self>) {
        self.initiate_shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flamix_core::model::{
        Credentials, ProviderRouting, Security, Server, Settings, Subscription,
    };

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
            state.subscriptions[1]
                .provider
                .routing
                .as_ref()
                .unwrap()
                .name,
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
        assert!(
            !state.subscriptions[1]
                .provider
                .routing
                .as_ref()
                .unwrap()
                .enabled
        );
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
        assert!(
            state.subscriptions[0]
                .provider
                .routing
                .as_ref()
                .unwrap()
                .enabled
        );
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

    #[test]
    fn proxy_marker_roundtrip() {
        let dir = std::env::temp_dir().join(format!("flamix-marker-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        assert!(!proxy_marker(&dir).exists());
        mark_proxy_set(&dir);
        assert!(proxy_marker(&dir).exists());
        clear_proxy_marker(&dir);
        assert!(!proxy_marker(&dir).exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn first_line_skips_blank_lines() {
        assert_eq!(first_line("\n\n  hello \nworld"), "hello");
        assert_eq!(first_line("   "), "");
        assert_eq!(first_line("core exited: boom\nmore"), "core exited: boom");
    }

    #[test]
    fn extract_import_handles_positional_and_flag() {
        assert_eq!(
            extract_import(&["--demo".to_string(), "flamix://x".to_string()]),
            Some("flamix://x".to_string())
        );
        assert_eq!(
            extract_import(&["--import".to_string(), "https://e/sub".to_string()]),
            Some("https://e/sub".to_string())
        );
        assert_eq!(
            extract_import(&[
                "--screenshot".to_string(),
                "out.png".to_string(),
                "https://e/sub".to_string()
            ]),
            Some("https://e/sub".to_string())
        );
        assert_eq!(extract_import(&["--demo".to_string()]), None);
    }

    #[test]
    fn failover_nodes_keeps_primary_first_and_dedupes() {
        let mut state = AppState {
            local_servers: vec![server("a"), server("b"), server("c")],
            selected_server: Some("a".to_string()),
            ..AppState::default()
        };
        state.settings.failover.enabled = true;
        state.settings.failover.backups = vec![
            "b".to_string(),
            "missing".to_string(),
            "a".to_string(),
            "c".to_string(),
            "b".to_string(),
        ];
        let ids: Vec<String> = failover_nodes(&state, "a")
            .iter()
            .map(|s| s.id.clone())
            .collect();
        assert_eq!(ids, vec!["a", "b", "c"]);
    }

    #[test]
    fn failover_nodes_empty_without_primary() {
        let state = AppState {
            local_servers: vec![server("b")],
            ..AppState::default()
        };
        assert!(failover_nodes(&state, "missing").is_empty());
    }

    #[test]
    fn plans_use_failover_only_for_real_clusters() {
        let mut settings = Settings::default();
        let primary = server("a");
        let compatible = server("b");
        // hysteria2 is sing-box only: Xray cannot proxy it, so `plan_cluster` drops it.
        let mut incompatible = server("hy2");
        incompatible.credentials = Credentials::Hysteria2 {
            password: "p".to_string(),
            obfs: None,
            obfs_password: None,
            ports: None,
            up_mbps: None,
            down_mbps: None,
        };
        let ctx = config::ConnectContext::default();

        // Failover off: the single `proxy` outbound, old stats pattern.
        assert!(!plans_use_failover(&config::plan_with(
            &primary, &settings, &ctx
        )));

        settings.failover.enabled = true;
        // Enabled but only an incompatible backup: the cluster collapses back to one node
        // (the single `proxy` outbound), so the proxy stats pattern is kept.
        let collapsed = vec![primary.clone(), incompatible];
        assert!(!plans_use_failover(&config::plan_cluster(
            &collapsed, &settings, &ctx
        )));

        // Primary + one compatible backup: real `node-N` outbounds.
        let nodes = vec![primary.clone(), compatible];
        assert!(plans_use_failover(&config::plan_cluster(
            &nodes, &settings, &ctx
        )));
    }

    #[test]
    fn probe_target_ids_dedupes_unknown_and_keeps_order() {
        let state = AppState {
            local_servers: vec![server("a"), server("b")],
            ..AppState::default()
        };
        let ids = vec![
            "b".to_string(),
            "missing".to_string(),
            "a".to_string(),
            "b".to_string(),
        ];
        assert_eq!(probe_target_ids(&state, &ids), vec!["b", "a"]);
    }

    #[test]
    fn probe_target_ids_empty_input_is_empty() {
        let state = AppState {
            local_servers: vec![server("a")],
            ..AppState::default()
        };
        assert!(probe_target_ids(&state, &[]).is_empty());
    }

    #[test]
    fn stale_probe_store_guard_rejects_wrong_epoch_or_port() {
        // The current connection: epoch 2, server "a", socks 10808.
        assert!(probe_store_is_current(2, 2, "a", "a", 10808, 10808));
        // A reconnect bumped the epoch: the old probe must not store.
        assert!(!probe_store_is_current(3, 2, "a", "a", 10808, 10808));
        // Same epoch but a different server id never matches.
        assert!(!probe_store_is_current(2, 2, "b", "a", 10808, 10808));
        // A reconnect that reused but changed the port must also be rejected.
        assert!(!probe_store_is_current(2, 2, "a", "a", 10809, 10808));
    }

    #[test]
    fn probe_progress_text_formats_counts() {
        assert_eq!(probe_progress_text(2, 12), "Проверка IP: 2/12 …");
        assert_eq!(probe_progress_text(0, 1), "Проверка IP: 0/1 …");
    }

    #[test]
    fn ticker_stays_only_for_its_own_epoch_and_server() {
        // The ticker from connect epoch 1 owns this connection.
        assert!(ticker_still_owns(1, 1, "a", "a"));
        // A same-id reconnect bumps the epoch: the old ticker must stop.
        assert!(!ticker_still_owns(2, 1, "a", "a"));
        // Different server id is also not ours.
        assert!(!ticker_still_owns(1, 1, "b", "a"));
    }
}

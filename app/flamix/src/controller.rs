//! UI controller: owns [`AppState`] and bridges the Slint callbacks to the core/sys crates.
//!
//! The UI thread never does I/O. Every callback either mutates the in-memory state and
//! persists it, or hands work to the multi-thread tokio runtime; background results are pushed
//! back with `Weak::upgrade_in_event_loop`.

use crate::view;
use crate::{flags, AppWindow, ConnState};
use flamix_core::model::{AppState, ConnectionMode};
use slint::{ComponentHandle, Weak};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// Everything needed to talk to the running cores (filled in when connected).
pub struct Connection {}

/// Bridges the Slint UI and the Flamix core.
pub struct Controller {
    pub(crate) state: Arc<Mutex<AppState>>,
    pub(crate) data_dir: PathBuf,
    pub(crate) rt: tokio::runtime::Handle,
    pub(crate) ui: Weak<AppWindow>,
    pub(crate) conn: Arc<Mutex<Option<Connection>>>,
    pub(crate) persist: crate::persist::Persist,
    pub(crate) sort_by_ping: AtomicBool,
}

impl Controller {
    /// Create the controller, load the persisted state and wire every UI callback.
    pub fn new(
        ui: &AppWindow,
        data_dir: PathBuf,
        rt: tokio::runtime::Handle,
        state: AppState,
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
        });
        controller.wire(ui);
        controller.apply_now();
        controller
    }

    /// Snapshot the state and paint the whole window. Must run on the UI thread.
    pub(crate) fn apply_to(&self, ui: &AppWindow) {
        let (subs, locals, name, iso, tun) = {
            let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            let query = ui.get_search_text().to_string();
            let sort = self.sort_by_ping.load(Ordering::Relaxed);
            (
                view::build_subscriptions(&state, &query, sort),
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

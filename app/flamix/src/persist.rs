//! Persistence of [`AppState`] for the controller.
//!
//! [`load`] delegates to `flamix_core::store`. Saving is debounced: after every mutation the
//! controller calls [`Persist::schedule`], and a background task writes at most one
//! `state.json` per second (coalescing intermediate states). On exit the caller uses
//! [`Persist::save_now`] for a synchronous write.

use flamix_core::model::AppState;
use flamix_core::{store, Result};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::sync::mpsc;

/// Minimum delay between two debounced writes.
const DEBOUNCE: Duration = Duration::from_secs(1);

/// Load `state.json` from `dir` (missing file = defaults, corrupt file is backed up).
pub fn load(dir: &Path) -> Result<AppState> {
    store::load(dir)
}

/// Handle to the debounced writer task.
pub struct Persist {
    dir: PathBuf,
    tx: mpsc::UnboundedSender<AppState>,
}

impl Persist {
    /// Start the background writer on `handle`. Owns `dir`.
    pub fn spawn(dir: PathBuf, handle: &tokio::runtime::Handle) -> Self {
        let (tx, mut rx) = mpsc::unbounded_channel::<AppState>();
        let task_dir = dir.clone();
        handle.spawn(async move {
            while let Some(mut state) = rx.recv().await {
                // Coalesce whatever is already queued, wait out the debounce window and take
                // the newest state that arrived meanwhile. Yields at most one write per second.
                while let Ok(newer) = rx.try_recv() {
                    state = newer;
                }
                tokio::time::sleep(DEBOUNCE).await;
                while let Ok(newer) = rx.try_recv() {
                    state = newer;
                }
                if let Err(err) = store::save(&task_dir, &state) {
                    tracing::warn!(%err, "failed to save state.json");
                }
            }
        });
        Self { dir, tx }
    }

    /// Queue a snapshot; the write happens asynchronously (debounced).
    pub fn schedule(&self, state: AppState) {
        let _ = self.tx.send(state);
    }

    /// Write `state` immediately (used on exit).
    pub fn save_now(&self, state: &AppState) -> Result<()> {
        store::save(&self.dir, state)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flamix_core::model::Server;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn temp_dir(tag: &str) -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let ns = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        std::env::temp_dir().join(format!("flamix-persist-{tag}-{}-{ns}-{n}", std::process::id()))
    }

    #[test]
    fn load_missing_dir_returns_defaults_and_creates_it() {
        let dir = temp_dir("missing");
        assert!(!dir.exists());
        let state = load(&dir).unwrap();
        assert!(state.subscriptions.is_empty());
        assert!(dir.is_dir());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_now_roundtrips() {
        let dir = temp_dir("roundtrip");
        let persist = Persist {
            dir: dir.clone(),
            tx: mpsc::unbounded_channel().0,
        };
        let mut state = AppState::default();
        state.settings.socks_port = 12345;
        state.local_servers.push(Server {
            id: "x".to_string(),
            name: "x".to_string(),
            country: None,
            address: "h".to_string(),
            port: 443,
            credentials: flamix_core::model::Credentials::Trojan {
                password: "p".to_string(),
            },
            transport: Default::default(),
            security: Default::default(),
            raw_link: None,
            traffic: None,
            local_traffic: Default::default(),
            last_ping_ms: None,
        });
        persist.save_now(&state).unwrap();
        let loaded = load(&dir).unwrap();
        assert_eq!(loaded.settings.socks_port, 12345);
        assert_eq!(loaded.local_servers, state.local_servers);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn scheduled_state_is_written_to_disk() {
        let dir = temp_dir("debounced");
        let persist = Persist::spawn(dir.clone(), &tokio::runtime::Handle::current());
        let mut state = AppState::default();
        state.settings.socks_port = 23456;
        persist.schedule(state);

        // The debounce window is 1 s; allow the writer task some slack.
        for _ in 0..40 {
            tokio::time::sleep(Duration::from_millis(100)).await;
            if let Ok(loaded) = load(&dir) {
                if loaded.settings.socks_port == 23456 {
                    let _ = std::fs::remove_dir_all(&dir);
                    return;
                }
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
        panic!("debounced write never landed");
    }
}

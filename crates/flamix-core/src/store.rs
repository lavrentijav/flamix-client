//! Persistence of [`AppState`]. OWNER: task T04.

use crate::model::AppState;
use crate::Result;
use std::path::{Path, PathBuf};

/// Per-user data directory: Linux `~/.local/share/flamix`, macOS
/// `~/Library/Application Support/online.flamix.Flamix`, Windows `%APPDATA%\Flamix\data`.
/// Overridable with env `FLAMIX_HOME` (portable mode / tests).
pub fn data_dir() -> PathBuf {
    todo!("T04")
}

/// Load `state.json` from `dir`; missing file = `AppState::default()`. A corrupt file is
/// renamed to `state.json.bad-<unix ts>` and defaults are returned (never lose the app start).
pub fn load(dir: &Path) -> Result<AppState> {
    let _ = dir;
    todo!("T04")
}

/// Atomically save: write `state.json.tmp`, fsync, rename over `state.json`.
pub fn save(dir: &Path, state: &AppState) -> Result<()> {
    let _ = (dir, state);
    todo!("T04")
}

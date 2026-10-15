//! Persistence of [`AppState`]. OWNER: task T04.

use crate::model::AppState;
use crate::Result;
use std::path::{Path, PathBuf};

/// Per-user data directory: Linux `~/.local/share/flamix`, macOS
/// `~/Library/Application Support/online.flamix.Flamix`, Windows `%APPDATA%\Flamix\data`.
/// Overridable with env `FLAMIX_HOME` (portable mode / tests).
pub fn data_dir() -> PathBuf {
    if let Some(home) = std::env::var_os("FLAMIX_HOME") {
        if !home.is_empty() {
            return PathBuf::from(home);
        }
    }
    match directories::ProjectDirs::from("online", "flamix", "Flamix") {
        Some(dirs) => dirs.data_dir().to_path_buf(),
        None => PathBuf::from("./flamix-data"),
    }
}

/// Load `state.json` from `dir`; missing file = `AppState::default()`. A corrupt file is
/// renamed to `state.json.bad-<unix ts>` and defaults are returned (never lose the app start).
///
/// If no device id is stored yet, a random one is generated and persisted right away so it
/// stays stable across loads.
pub fn load(dir: &Path) -> Result<AppState> {
    std::fs::create_dir_all(dir)?;
    let path = dir.join("state.json");

    let mut state = match std::fs::read(&path) {
        Ok(bytes) => match serde_json::from_slice::<AppState>(&bytes) {
            Ok(state) => state,
            Err(err) => {
                tracing::warn!("state.json is corrupt ({err}); backing it up");
                let backup = dir.join(format!("state.json.bad-{}", unix_now()));
                if let Err(err) = std::fs::rename(&path, &backup) {
                    tracing::warn!("could not move corrupt state.json aside: {err}");
                }
                AppState::default()
            }
        },
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => AppState::default(),
        Err(err) => return Err(err.into()),
    };

    let from = state.version;
    migrate(&mut state, from);

    let generated_device_id = if state.device_id.is_none() {
        state.device_id = Some(generate_device_id());
        true
    } else {
        false
    };
    state.version = AppState::VERSION;

    if generated_device_id {
        if let Err(err) = save(dir, &state) {
            tracing::warn!("could not persist the generated device id: {err}");
        }
    }

    Ok(state)
}

/// Atomically save: write `state.json.tmp`, fsync, rename over `state.json`.
pub fn save(dir: &Path, state: &AppState) -> Result<()> {
    use std::io::Write;

    std::fs::create_dir_all(dir)?;
    let tmp = dir.join("state.json.tmp");
    let target = dir.join("state.json");

    let data = serde_json::to_vec_pretty(state)?;
    {
        let mut file = std::fs::File::create(&tmp)?;
        file.write_all(&data)?;
        file.sync_all()?;
    }
    std::fs::rename(&tmp, &target)?;
    Ok(())
}

/// Schema migrations. Currently a no-op; future versions upgrade `state` here based on
/// `from` (the version read from disk). `state.version` is set to [`AppState::VERSION`]
/// by [`load`] afterwards.
fn migrate(state: &mut AppState, from: u32) {
    let _ = (state, from);
}

/// Random, stable-ish device id (16 random bytes, hex). Only uniqueness matters, so a hash of
/// `RandomState` entropy + wall clock + pid is enough — no crypto RNG dependency.
fn generate_device_id() -> String {
    use sha2::{Digest, Sha256};
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};
    use std::time::{SystemTime, UNIX_EPOCH};

    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());

    let mut seed = RandomState::new().build_hasher();
    seed.write_u128(nanos);
    seed.write_u32(std::process::id());
    let entropy = seed.finish();

    let mut hasher = Sha256::new();
    hasher.update(entropy.to_le_bytes());
    hasher.update(nanos.to_le_bytes());
    hasher.update(std::process::id().to_le_bytes());
    hex::encode(&hasher.finalize()[..16])
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn temp_dir(tag: &str) -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let ns = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        std::env::temp_dir().join(format!(
            "flamix-store-{tag}-{}-{ns}-{n}",
            std::process::id()
        ))
    }

    fn cleanup(dir: &Path) {
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn roundtrip() {
        let dir = temp_dir("roundtrip");
        let mut state = AppState::default();
        state.settings.socks_port = 12345;
        state.settings.ping_url = "https://example.com/generate_204".into();
        state.local_servers.push(crate::model::Server {
            id: "abc123".into(),
            name: "test".into(),
            country: None,
            address: "example.com".into(),
            port: 443,
            credentials: crate::model::Credentials::Trojan {
                password: "secret".into(),
            },
            transport: Default::default(),
            security: Default::default(),
            raw_link: None,
            traffic: None,
            local_traffic: Default::default(),
            last_ping_ms: None,
        });

        save(&dir, &state).unwrap();
        let loaded = load(&dir).unwrap();

        assert_eq!(loaded.settings.socks_port, 12345);
        assert_eq!(loaded.settings.ping_url, state.settings.ping_url);
        assert_eq!(loaded.local_servers, state.local_servers);
        cleanup(&dir);
    }

    #[test]
    fn corrupt_file_is_backed_up() {
        let dir = temp_dir("corrupt");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("state.json"), b"{ this is not json").unwrap();

        let loaded = load(&dir).unwrap();
        assert!(loaded.device_id.is_some());

        let backups = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.file_name()
                    .to_string_lossy()
                    .starts_with("state.json.bad-")
            })
            .count();
        assert_eq!(
            backups, 1,
            "corrupt state should be moved aside exactly once"
        );
        cleanup(&dir);
    }

    #[test]
    fn missing_dir_is_created() {
        let dir = temp_dir("missing");
        assert!(!dir.exists());

        let loaded = load(&dir).unwrap();
        assert!(dir.is_dir());
        assert!(loaded.device_id.is_some());
        cleanup(&dir);
    }

    #[test]
    fn device_id_stable_across_loads() {
        let dir = temp_dir("device");

        let first = load(&dir).unwrap();
        let second = load(&dir).unwrap();

        let id = first.device_id.unwrap();
        assert_eq!(id.len(), 32);
        assert!(id.chars().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(second.device_id.unwrap(), id);
        cleanup(&dir);
    }
}

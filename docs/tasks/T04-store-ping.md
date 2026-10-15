# T04 — state store + latency tests

Read `AGENTS.md` first (build rule: everything via `scripts/ru1`). Contract: `crates/flamix-core/src/model.rs`.

## Setup
    git -C /srv/dev/projects/flamix-client worktree add /srv/dev/projects/flamix-client-wt/T04 -b oc/T04
    cd /srv/dev/projects/flamix-client-wt/T04

## Files you own
`crates/flamix-core/src/store.rs`, `crates/flamix-core/src/ping.rs`.

## store.rs
- `data_dir()`: per doc comment, via `directories::ProjectDirs::from("online", "flamix", "Flamix")`
  (`data_dir()`), `FLAMIX_HOME` env wins. Fallback if ProjectDirs is None: `./flamix-data`.
- `load(dir)`: as documented. After loading: if `state.device_id` is None, generate 16 random bytes
  hex (no new crate: use `std::collections::hash_map::RandomState` + time + pid hashed via sha2, it
  only has to be unique, not cryptographic); set `version = AppState::VERSION` (migrations hook:
  a `fn migrate(state: &mut AppState, from: u32)` that does nothing yet). Create `dir` if missing.
- `save(dir, state)`: atomic as documented (`File::sync_all` before rename; on Windows `rename`
  over an existing file works with `std::fs::rename`). Pretty JSON.
- Also add `pub fn save_debounced`? NO — keep the API minimal.
- Also add `pub fn import_v2rayn_backup`? NO.

## ping.rs
- `tcp_ping`: resolve first (tokio::net::lookup_host, not timed), then time `TcpStream::connect`
  with `tokio::time::timeout`. Return ms rounded up (min 1).
- `url_ping`: reqwest client with `Proxy::all(proxy)`, no redirects, timeout; one warm-up GET
  (ignored), then a timed GET (time until headers). Status 2xx/204/3xx counts as success.
- Add `pub async fn tcp_ping_many(targets: Vec<(String, String, u16)>, timeout, concurrency: usize) -> Vec<(String, Option<u32>)>`
  (id, host, port) using a `tokio::sync::Semaphore` / `JoinSet` — used by "test all".

## Tests
store: roundtrip in a temp dir (use `std::env::temp_dir()` + unique subdir, clean up), corrupt file
backed up, missing dir created, device id stable across loads. ping: local `TcpListener` -> Some,
closed port -> None, unroutable `10.255.255.1` with 300 ms timeout -> None; url_ping against a local
minimal HTTP proxy is NOT required (test that a dead proxy returns None quickly).

## Done
    scripts/ru1 cargo fmt --all --check
    scripts/ru1 cargo clippy -p flamix-core --all-targets -- -D warnings
    scripts/ru1 cargo test -p flamix-core store ping
Commit on `oc/T04`. Report: branch, commit, test results.

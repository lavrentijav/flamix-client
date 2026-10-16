# T20 — backup & restore (export/import all state)

Read `AGENTS.md` first (build rule: `scripts/ru1`; work incrementally, one file at a time).
Feature from `docs/research/gaps.md` #5 (high value): let the user export everything Flamix stores
(subscriptions, local servers, settings, routing, traffic stats) to one JSON file and import it back
on the same or another machine. No cloud, no new state model — just a portable snapshot.

## Setup
    git -C /srv/dev/projects/flamix-client worktree add /srv/dev/projects/flamix-client-wt/T20 -b oc/T20
    cd /srv/dev/projects/flamix-client-wt/T20

## Files you own
- `crates/flamix-core/src/backup.rs` (new) + one line in `crates/flamix-core/src/lib.rs` to declare it.
- `app/flamix/src/pages/about.rs`, `app/flamix/ui/pages/about-page.slint`.
- `app/flamix/Cargo.toml` (only to add the `rfd` dependency below).
Nothing else. `model.rs`, `store.rs`, `controller.rs` and other pages are off-limits — the existing
`Controller::mutate(|state| ...)` / `with_state` page API is enough.

## Core: `crates/flamix-core/src/backup.rs`
The whole state is already `serde`-serialisable (`model::AppState`). Add a self-describing wrapper so a
foreign JSON file is rejected cleanly:

```rust
pub const FORMAT: &str = "flamix-backup";
pub const VERSION: u32 = 1;   // bump only on an incompatible format change

#[derive(serde::Serialize, serde::Deserialize)]
pub struct Backup {
    pub format: String,      // must equal FORMAT
    pub version: u32,        // <= VERSION
    pub app_version: String, // env!("CARGO_PKG_VERSION") of the writer
    pub exported_at: i64,    // unix seconds
    pub state: flamix_core::model::AppState,
}
```

Public API (return the existing `flamix_core::Result`):
- `pub fn export(state: &AppState) -> Result<String>` — clone the state, set `device_id = None`
  (never leak the local `x-hwid` device id into a file the user may share), wrap it and return pretty JSON.
- `pub fn import(text: &str) -> Result<AppState>` — parse the wrapper; return `Error::Parse(...)` if
  `format != FORMAT` ("это не резервная копия Flamix") or `version > VERSION` ("копия новее этой версии
  приложения"); on success return `state` with `device_id = None`.
- `pub fn apply_imported(current: &mut AppState, imported: AppState)` — replace every field of `current`
  from `imported` **except** `device_id` (keep the machine's own id) and force `version = AppState::VERSION`.

Rules: no `unwrap`/`expect` on the file/JSON data; every function returns `Result`. Comments in English.
Unit tests (they may `unwrap`): round-trip equality of an `AppState` with subscriptions + local servers +
settings; `device_id` stripped on export and ignored on import; wrong `format` and `version = VERSION+1`
both rejected; `apply_imported` keeps the local device id and adopts the imported subscriptions/settings.

## App: "Резервная копия" section on the About page
Use the native file dialog crate `rfd` (add to `app/flamix/Cargo.toml`):

    rfd = { version = "0.15", default-features = false, features = ["xdg-portal"] }

(`xdg-portal` keeps Linux free of a GTK build dependency; Windows/macOS use their native dialogs.
If the pinned version/feature does not build, pick the smallest working set and say so in the report.)

- `AboutBridge`: add `in property <string> backup-status` and two callbacks `on_export` / `on_import`
  (plus a confirm step for import: `in-out property <bool> import-confirm`, `on_import_confirmed`,
  `on_import_cancelled` — import overwrites the current state, so it must be confirmed).
- Export: `rfd::FileDialog::new().add_filter("Flamix backup", &["json"]).set_file_name("flamix-backup.json").save_file()`
  → `flamix_core::backup::export(&state)` → write the file → status "Сохранено: <путь>" or an error line.
  Read the state with `ctrl.with_state(...)`.
- Import: pick a file → read + `flamix_core::backup::import(...)`; on success show the confirm row
  ("Импортировать копию и заменить текущие настройки и серверы?"); on confirm call
  `ctrl.mutate(|s| flamix_core::backup::apply_imported(s, imported))` then status "Импортировано"; on error
  show the error. Wait for `ctrl.mutate` to persist — do not do file I/O under the state lock.
- Demo/`--screenshot` mode (`ctrl == None`): the buttons fill a demo status line and no-op.
- Follow the existing About page style: a card with 8px radius (`Theme.radius`) + 1px stroke, page title
  style of T17A, Russian labels ("Экспортировать…", "Импортировать…", "Отмена").

## Done
    scripts/ru1 cargo fmt --all --check
    scripts/ru1 cargo clippy --workspace --all-targets -- -D warnings
    scripts/ru1 cargo test -p flamix-core -p flamix
    scripts/ru1 scripts/check-windows.sh
    FLAMIX_FETCH=shots scripts/ru1 sh -c 'cargo run -p flamix -- --demo --screenshot shots/about.png --page 5'
Regenerate `docs/design/screenshot-about.png` and `docs/design/screenshot-about-light.png` and commit them.
Commit on `oc/T20` with meaningful messages. Report: branch, commit hash, test counts, the `rfd`
feature actually used, and anything unfinished. Do not push, do not merge.

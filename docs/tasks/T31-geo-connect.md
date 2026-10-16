# T31 — geo databases: wire the download into the app (sys asset dir + controller)

Read `AGENTS.md` first (build rule: `scripts/ru1`; work incrementally).
`flamix_core::geo` now has `file_paths`, `is_fresh`, `download_file`, `ensure(dir, &GeoData, max_age)
-> GeoStatus` and resolves the URLs (`Settings.geo`, default Freedom-for-Russia). This task makes the
running cores actually use the downloaded `geosite.dat`/`geoip.dat` and refreshes them.

## Setup
    git -C /srv/dev/projects/flamix-client worktree add /srv/dev/projects/flamix-client-wt/T31 -b oc/T31
    cd /srv/dev/projects/flamix-client-wt/T31

## Files you own
`crates/flamix-sys/src/runner.rs`, `app/flamix/src/controller.rs`. Nothing else.

## A. sys: point Xray at the geo directory
Xray resolves `geosite:…`/`geoip:…` from `.dat` files in the directory named by the
`XRAY_LOCATION_ASSET` env var. Thread an optional geo dir through the runner:
- Change `pub async fn start(plans, work_dir)` to
  `pub async fn start(plans, work_dir, geo_dir: Option<&Path>)` (update its callers/tests).
- For **Xray** processes only, set `XRAY_LOCATION_ASSET=<geo_dir>`:
  - normal spawn (`spawn_command`): set the env in the `tokio::process::Command`;
  - elevated spawn: pass it into the watchdog script (add an `export XRAY_LOCATION_ASSET=…;` line, or
    a generic `env` prefix) so the root-launched core gets it too.
  - when `geo_dir` is `None`, do not set the variable (behaviour unchanged; the core falls back to the
    dir next to its binary).
- Keep sing-box unchanged (its config uses remote `.srs` rule-sets; no local geo path needed — note
  this in the report).
- Update `elevated_watchdog_script(..)` signature accordingly and its direct tests.

## B. controller: refresh before connect + a manual update
- Add a geo dir: `self.data_dir.join("geo")`.
- **Before** `runner::start` in `connect`, best-effort refresh:
  `geo::ensure(&geo_dir, &settings.geo, GEO_MAX_AGE).await` (define e.g. `GEO_MAX_AGE = 7*24*3600`);
  on `Err` **do not fail the connection** — log `tracing::warn!` and continue (a previously downloaded
  file stays usable; if none exists Xray will report the missing geo). Then call `runner::start(&plans,
  &run_dir, Some(&geo_dir))`. Update the TUN path the same way (same `start` call).
- On a successful refresh (`GeoStatus.downloaded`), store the time: `mutate(|s| s.geo_updated = now)`
  (unix seconds; reuse the existing `today_key`-style time source or `SystemTime`).
- Add a public method for the settings button:
  `pub fn update_geo(self: &Arc<Self>)` — refuses (status text) if a core is running, otherwise spawns
  on the runtime a **forced** refresh (`ensure(&geo_dir, &settings.geo, 0)`); on success set
  `geo_updated` and status "Геоданные обновлены", on error status "Ошибка обновления геоданных: …".
  Guard against concurrent runs (an `AtomicBool`, like the exit-IP probe) and clear it in all paths.
- The status/label uses Russian text consistent with the rest of the app.
- If `runner::start`'s signature change ripples into tests in this file, update them (use `None` where
  a geo dir is irrelevant).

## Constraints
- No `unwrap`/`expect` on external data; never let a geo download failure break connect.
- Comment the `GEO_MAX_AGE` choice and the best-effort behaviour. `-D warnings` clean.
- Do not touch `flamix-core` (the geo module is done) or the settings UI (a separate task).

## Done
    scripts/ru1 cargo fmt --all --check
    scripts/ru1 cargo clippy --workspace --all-targets -- -D warnings
    scripts/ru1 cargo test --workspace
    scripts/ru1 scripts/check-windows.sh
Commit on `oc/T31`. Report: branch, commit, the exact env/asset wiring, the download-failure
behaviour, test counts. Do not push, do not merge.

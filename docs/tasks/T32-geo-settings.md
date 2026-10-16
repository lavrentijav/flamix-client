# T32 — Settings page: a "Геоданные" section (source picker + update)

Read `AGENTS.md` first (build rule: `scripts/ru1`; work incrementally).
`Settings.geo` (`GeoData { source: GeoSource, geosite_url, geoip_url }`) and the controller's
`update_geo()` / `AppState.geo_updated` exist (T31). Add the UI so the user can change **what is
downloaded** and refresh it.

## Setup
    git -C /srv/dev/projects/flamix-client worktree add /srv/dev/projects/flamix-client-wt/T32 -b oc/T32
    cd /srv/dev/projects/flamix-client-wt/T32

## Files you own
`app/flamix/ui/pages/settings-page.slint`, `app/flamix/src/pages/settings.rs`. Nothing else.

## Do
1. **New "Геоданные" card** (after DNS / near TUN; use the existing `SettingsCard`/`SettingsField`
   style, 8px radius, Russian labels):
   - **Источник** ComboBox: `Freedom for Russia` / `Loyalsoldier` / `Свой URL`
     (`GeoSource::{FreedomForRussia, Loyalsoldier, Custom}`).
   - When `Свой URL`: two `LineEdit`s — **geosite.dat URL** and **geoip.dat URL** (validate they are
     empty or `http(s)://…`; show an inline error otherwise).
   - A hint explaining the source selects which RU/geo lists the cores use (default Freedom for Russia).
   - A **"Обновить геоданные"** button and a **"Обновлено: <date>"** label (from
     `AppState.geo_updated`, unix seconds → local date/time; "никогда" when 0). The button calls
     `ctrl.update_geo()` (no-op with a demo status line when `ctrl == None`). After clicking, refresh
     the label from state (the controller updates it asynchronously — re-read on a short timer or when
     the page is shown; a 2 s `slint::Timer` like the stats page is fine).
2. **Bridge wiring**: add `geo-source-index`, `geo-geosite-url`, `geo-geoip-url`, `geo-updated-text`,
   `geo-urls-error`, a `callback update-geo()`, and `in-out property <bool> advanced`-style visibility
   as needed. Implement `source_index`/`source_from_index` mapping helpers with unit tests. Extend
   `write_bridge`/`read_bridge`/`apply_owned_in_place`/`merge_owned` to carry `geo` (like `dns`).
   Changing the source or a custom URL should mark the data stale so the next connect re-downloads:
   set `state.geo_updated = 0` via the existing save path (or leave `geo_updated` and rely on it being
   refreshed on connect — document your choice). Add `geo` to the core-affecting comparison so a source
   change triggers `reconnect_if_connected` (like the other networking settings).
3. **Demo mode**: fill a realistic source + a static "Обновлено: <date>" so the screenshot shows the card.

## Constraints
- Do not edit the controller/sys/core (their API is fixed: `ctrl.update_geo()`, `Settings.geo`).
- No `unwrap`/`expect` on user input; validate URLs; comments in English; `-D warnings` clean.

## Done
    scripts/ru1 cargo fmt --all --check
    scripts/ru1 cargo clippy --workspace --all-targets -- -D warnings
    scripts/ru1 cargo test -p flamix
    FLAMIX_FETCH=shots scripts/ru1 sh -c 'mkdir -p shots && cargo run -p flamix -- --demo --screenshot shots/settings.png --page 3 && cargo run -p flamix -- --demo --light --screenshot shots/settings-light.png --page 3'
Regenerate `docs/design/screenshot-settings.png` / `-settings-light.png`. Commit on `oc/T32`. Report:
branch, commit, mapping/staleness decisions, test counts. Do not push, do not merge.

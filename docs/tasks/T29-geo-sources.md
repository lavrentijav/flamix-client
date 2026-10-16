# T29 — geo data: source → download URLs (Freedom for Russia default), wire into the configs

Read `AGENTS.md` first (build rule: `scripts/ru1`; work incrementally). Read-only web access is allowed
for the two geo repos (see below); no installs, no POST.
Context: the owner wants the default rule data to be the **"Freedom for Russia"** lists
(`runetfreedom/russia-v2ray-rules-dat`), mainly for Russian users, with a setting to change what is
downloaded later. The model already has `Settings.geo: GeoData` (`GeoSource::{FreedomForRussia (default),
Loyalsoldier, Custom}`, `geosite_url`, `geoip_url`) — do not change `model.rs`.

## Setup
    git -C /srv/dev/projects/flamix-client worktree add /srv/dev/projects/flamix-client-wt/T29 -b oc/T29
    cd /srv/dev/projects/flamix-client-wt/T29

## Files you own
`crates/flamix-core/src/geo.rs` (new), `crates/flamix-core/src/lib.rs` (declare the module),
`crates/flamix-core/src/config/**`. Nothing else.

## Do
1. **`geo.rs`** — resolve the download URLs for a source (pure, no network in the library):
   - `pub struct GeoUrls { pub xray_geosite: String, pub xray_geoip: String, pub singbox_geosite_base: String, pub singbox_geoip_base: String }`
   - `pub fn urls(geo: &GeoData) -> GeoUrls` using the `GeoSource`:
     - `FreedomForRussia` → the `runetfreedom/russia-v2ray-rules-dat` release assets for `geosite.dat` /
       `geoip.dat` (Xray) and, if the project publishes sing-box `.srs` rule-sets, their base URLs; if it
       does not, use the SagerNet bases for sing-box and say so in the report.
     - `Loyalsoldier` → `Loyalsoldier/v2ray-rules-dat` `.dat` assets + SagerNet `.srs` bases.
     - `Custom` → `geo.geosite_url` / `geo.geoip_url` for Xray; sing-box falls back to the SagerNet bases
       with a `tracing::warn!` (custom `.dat` cannot be turned into `.srs`).
   - **Verify** the exact, current URLs by fetching the repos' release pages / API (read-only):
     <https://github.com/runetfreedom/russia-v2ray-rules-dat>, <https://github.com/Loyalsoldier/v2ray-rules-dat>,
     <https://github.com/SagerNet/sing-geosite>, <https://github.com/SagerNet/sing-geoip>.
     Pin a concrete release tag in the constant (not `latest`), matching how `scripts/fetch-cores.sh` pins
     versions. Document each URL with a comment and the tag.
   - **Verify the category names** used by our RU preset exist in the chosen source: our preset uses
     `geosite:category-ru`, `geosite:private`, `geosite:category-ads-all`, `geoip:ru`, `geoip:private`
     (`crates/flamix-core/src/config/routing.rs`). Confirm these names exist in the Freedom-for-Russia
     lists (fetch the repo's rule list / README); if a name differs, say exactly what it is in the report.
   - Unit tests: URL mapping for all three sources, custom URL passthrough, and that the default is
     `FreedomForRussia`.
2. **sing-box config** (`config/singbox.rs`): replace the hardcoded `GEOSITE_BASE` / `GEOIP_BASE` constants
   with the bases from `geo::urls(&settings.geo)`. The sing-box generators already receive `settings`
   (thread it to the `rule_set` builder / `geosite_url(name)` / `geoip_url(name)` helpers). A provider
   custom `.srs` URL (`GeoUrls::from_provider`, existing) still wins over the settings source.
3. **Xray config** (`config/xray.rs` / `config/mod.rs`): nothing downloads inside the config; just make
   sure the `geosite:`/`geoip:` names stay unchanged (they are resolved from the local `.dat` provided by
   the app later — that is a separate task). If a source lacks a name, do not silently drop it.
4. Keep default behaviour with `GeoSource::FreedomForRussia`; the existing real-core sing-box tests must
   still pass (remote `.srs` URLs are not fetched by `sing-box check`).

## Done
    scripts/ru1 cargo fmt --all --check
    scripts/ru1 cargo clippy --workspace --all-targets -- -D warnings
    scripts/ru1 sh -c 'FLAMIX_XRAY_BIN=$HOME/flamix-tools/cores/xray FLAMIX_SINGBOX_BIN=$HOME/flamix-tools/cores/sing-box cargo test --workspace -- --include-ignored'
Commit on `oc/T29`. Report: branch, commit, the **exact pinned URLs** you used per source, whether
Freedom-for-Russia publishes sing-box `.srs` (and what base you used if not), the category-name check
result, and test counts. Do not push, do not merge.

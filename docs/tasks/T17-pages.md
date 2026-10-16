# T17 — secondary pages (four parallel agents: T17A Add+About, T17B Settings, T17C Routing, T17D Statistics)

Read `AGENTS.md` first (build rule: `scripts/ru1`; work incrementally). Your prompt tells you which letter you are —
do ONLY that section. Several agents work in parallel, so file ownership is strict.

## Setup (replace X with your letter)
    git -C /srv/dev/projects/flamix-client worktree add /srv/dev/projects/flamix-client-wt/T17X -b oc/T17X
    cd /srv/dev/projects/flamix-client-wt/T17X

## How pages are built (already scaffolded in main)
- Each page = `app/flamix/ui/pages/<name>-page.slint` (component `<Name>Page` + `export global <Name>Bridge`) and
  `app/flamix/src/pages/<name>.rs` (`pub fn init(ui: &AppWindow, ctrl: Option<&Arc<Controller>>)`).
- Rust talks to the page ONLY through the bridge global: `ui.global::<NameBridge>()` — properties (`in`/`in-out`) and
  callbacks (`on_...`). `ctrl == None` means demo/screenshot mode: fill realistic demo data, callbacks may no-op.
- Controller API for pages (`app/flamix/src/controller.rs`, block "API for the secondary pages"): `with_state(|s| ..)`,
  `mutate(|s| ..)` (persists + re-renders), `import(text)`, `is_connected()`, `reconnect_if_connected()`, `runtime()`,
  `status(text)`. Do not edit controller.rs; if you need more API, implement it inside your page module on top of these
  (e.g. call `flamix_core`/`flamix_sys` functions directly) and mention it in the report.
- Model: `crates/flamix-core/src/model.rs` (`Settings`, `Routing`, `RoutingRule`, `Subscription`, `ProviderExtras`, `daily_traffic`, ...).
- Look & feel: reuse `ui/theme.slint` (global `Theme`: colours, radii, fonts), `ui/components/icon.slint` and the
  existing components; Fluent/WinUI 3 style like the servers panel: page title 20–28px, cards with 8px radius and 1px
  stroke, 12/14px text, `std-widgets` (LineEdit, Switch, ComboBox, SpinBox, Button, ScrollView, TextEdit) with the
  fluent style, Russian labels. Page fills the area between the rail and the connection panel, scrolls if long.
- Screenshot proof (required): `FLAMIX_FETCH=shots scripts/ru1 sh -c 'cargo run -p flamix -- --demo --screenshot shots/<name>.png --page <N>'`
  (pages: 1 add, 2 routing, 3 settings, 4 stats, 5 about) and the same with `--light` → commit as
  `docs/design/screenshot-<name>.png` / `screenshot-<name>-light.png`. You cannot see images — the lead reviews them.

## Files you own
ONLY your page's `.slint` file(s) (you may add `ui/pages/<name>-*.slint` helper components) and your `src/pages/<name>.rs`
(+ `src/pages/<name>/` submodules). Nothing else. No new crates except where your section says so.

## T17A — "Добавить" (page 1) + "О программе" (page 5)
Add: (1) "Подписка": URL field, optional name, optional backup URLs (multi-line), button "Добавить" → `ctrl.import(url)`
then, after it appears in state, set name/backup URLs via `mutate` (match by url). (2) "Ссылки": multi-line TextEdit for share
links → `ctrl.import(text)`. (3) "Из буфера обмена" button (same as the servers panel's import). (4) Short help text listing
accepted formats (vless/vmess/trojan/ss/hy2/tuic/wg, subscription URLs, flamix:// happ:// v2raytun:// links). Validation:
empty → disabled button; status/result line on the page. After success navigate to page 0 (`ui.set_page(0)`).
About: app icon (`assets/app-icon.svg`), name, version (`env!("CARGO_PKG_VERSION")`), one-line description, buttons/links:
GitHub repo, Releases, Issues (open with `open` crate or `std::process::Command` per OS: `xdg-open` / `open` / `cmd /C start`),
licenses list (Flamix GPL-3.0-or-later; Xray-core MPL-2.0; sing-box GPL-3.0; Slint (royalty-free/GPL); Alfa-Bank icons MIT),
core versions (run `xray version` / `sing-box version` via `flamix_sys::runner::find_core` on a background task; "не найдено"
if missing).

## T17B — "Настройки" (page 3)
Sections (cards) bound to `Settings`: Подключение (mode Proxy/TUN, socks/http ports SpinBox 1024–65535, allow LAN, set system
proxy, bypass list multi-line, mux, default fingerprint ComboBox chrome/firefox/safari/edge/random/ios/android); TUN (stack
system/gvisor/mixed, MTU, IPv6); DNS (remote, direct, fake-ip); Пинг (method tcp/url, url); Приложение (launch at login,
connect on launch, minimize to tray, auto-update subscriptions, theme System/Dark/Light → also apply to `Theme.dark`
immediately, language ru/en (stored only), log level). Save on change (debounce 500 ms) via `mutate`; validation (ports
distinct, valid URL) with inline error text; changes affecting the running core → `reconnect_if_connected()` (ports, mode,
LAN, DNS, TUN, mux, fingerprint); `launch_at_login` → `flamix_sys::autostart::set_enabled` on a background task. A "Сбросить"
button restores `Settings::default()` after a confirm row.

## T17C — "Маршрутизация" (page 2)
(1) Preset selector (Global / Обход РФ / Свои правила) as three large selectable cards with one-line descriptions.
(2) Custom rules list: each row = action chip (Прокси/Напрямую/Блок), kind (Домены / IP / Процессы), values (comma separated,
Xray syntax hints like `geosite:category-ru`, `domain:example.com`, `geoip:ru`, `10.0.0.0/8`, `telegram.exe`), enabled Switch,
delete; "Добавить правило"; reorder up/down. (3) Domain strategy ComboBox (AsIs / IPIfNonMatch / IPOnDemand), Block QUIC switch.
(4) "Маршрутизация провайдера": for each subscription with `provider.routing` — name, rule count, enabled Switch
(writes `provider.routing.enabled`) and "Игнорировать" Switch (`options.ignore_provider_routing`); empty state text.
All edits via `mutate` + `reconnect_if_connected()` (debounced 700 ms).

## T17D — "Статистика" (page 4)
(1) Header cards: session traffic (from the connection panel speed is not enough — compute from `daily_traffic[today]` and
`local_traffic`), today, last 30 days total. (2) Bar chart of the last 30 days (`daily_traffic`, upload+download stacked,
two accent shades, hover/tap shows the day's numbers) drawn with Slint Rectangles (no images). (3) Top servers by
`local_traffic` (name, flag via `crate::flags::flag`, bytes, share bar). (4) Subscriptions: quota used/total, expiry, per-server
quotas count. (5) "Сбросить статистику" with confirm → clears `daily_traffic` and every `local_traffic`. Refresh every 2 s
while the page is visible (timer in Rust, `slint::Timer`). Human byte formatting: reuse `crate::view` helpers if public, else a
local helper with tests.

## Done (all)
    scripts/ru1 cargo fmt --all --check
    scripts/ru1 cargo clippy --workspace --all-targets -- -D warnings
    scripts/ru1 cargo test -p flamix
+ the two screenshots per page committed. Unit tests for every pure helper (validation, formatting, rule conversion, chart scaling).
Commit on `oc/T17X`. Report: branch, commit, screenshots paths, any controller API you missed.

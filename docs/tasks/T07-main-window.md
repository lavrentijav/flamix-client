# T07 — main window UI (Slint), matching the mockup, WinUI3 look

Read `AGENTS.md` first (build rule: everything via `scripts/ru1`).
**Look at `docs/design/main-window.png`** — the desired main window (layout and colours). Style: Windows 11 /
WinUI 3 (Fluent: Segoe-like typography, 4–8 px rounded corners, subtle 1 px strokes, layered "Mica"-like
panels, smooth hover/press states, 150–250 ms animations), with the warm brown/orange palette of the mockup.

## Setup
    git -C /srv/dev/projects/flamix-client worktree add /srv/dev/projects/flamix-client-wt/T07 -b oc/T07
    cd /srv/dev/projects/flamix-client-wt/T07
Icons/flags come from task T06 (branch `oc/T06`, merged into main before you start): `app/flamix/assets/icons/*.svg`
(monochrome, recolour with `colorize`), `app/flamix/assets/flags/<ISO>.svg`. Read `app/flamix/assets/icons/SOURCES.md`.

## Files you own
`app/flamix/ui/**` (split into `theme.slint`, `components/*.slint`, `pages/*.slint`, `app.slint`), `app/flamix/src/main.rs`,
new `app/flamix/src/demo.rs`, `app/flamix/src/flags.rs`, `app/flamix/build.rs`, `app/flamix/Cargo.toml` (only if needed).

## Layout (from the mockup, default window 960×600, min 720×480, resizable)
1. **Left rail** (56 px, collapsible to labels 200 px with the top arrow): buttons top→down: expand/collapse,
   Add (`add`), Routing (`routing`), Settings (`settings`), Statistics (`stats`); bottom: About (`info`).
   Selected page = highlighted pill + accent indicator bar (WinUI NavigationView style). Tooltips/labels in Russian.
2. **Servers panel** (flexible width, rounded large panel): title "Серверы"; search field (filters by name,
   case-insensitive) + edit button (`edit`, toggles edit mode: shows delete/move controls) + more button (`more`,
   menu: "Обновить все подписки", "Пинг всех", "Импорт из буфера", "Сортировать по пингу").
   Then a scrollable list of **subscription cards**:
   - header: chevron (collapse), name, actions `pin` (filled when pinned), `refresh` (spins while updating), `settings`;
   - traffic bar: info badge + progress bar "15GB / 100GB" (used / total, human readable; hidden when no data;
     "∞" when unlimited), expiry "до 01.01.2027" if present; announce text if present (small, muted);
   - server rows: flag (from ISO code, fallback `UN`), name, subtitle "vless / ws / tls", ping badge on the right
     ("42 ms" green <150, yellow <400, red else, "—" failed, empty untested), **per-server traffic**: when the server has
     its own quota draw a thin 3 px bar under the subtitle + "осталось 12.3 GB"; chevron-right opens server details
     (callback only). Selected server row highlighted with accent. Click = select.
   - "Локальные серверы" card for manually added servers (same row component, no traffic bar).
   - Empty state: illustration-free text "Нет серверов" + button "Добавить подписку".
3. **Connection panel** (right, ~300 px): big circular power button (~150 px, ring + `power` glyph) with states:
   disconnected (outline, muted), connecting (rotating arc animation), connected (filled accent, glow);
   below: flag + selected server name, "Тест пинга" button (shows result "· 42 ms"), Proxy/TUN segmented control,
   connection status text and live speed "↑ 1.2 MB/s ↓ 3.4 MB/s" + session traffic when connected.
4. Other pages (Add, Routing, Settings, Statistics, About): for this task just a placeholder page with the title;
   they are a separate task. Navigation between them must work.

## Theme (`theme.slint`, a global `Theme`)
Dark (default, from mockup): window bg `#2B2621`, rail `#2B2621`, panel `#3D3328`, card `#5A4632` header / `#4A3B2C` body,
accent `#C07A2C`, accent-strong `#8A5418`, text `#F2E9DF`, muted `#B9A894`, stroke `#FFFFFF14`, success `#6CCB5F`,
warning `#E3B341`, danger `#E5534B`. Light variant: same accents on `#F6F1EB` / `#FFFFFF` / text `#2B2621`.
`Theme.dark: bool` property switched from Rust. Font: default system font (Segoe UI Variable on Windows), sizes 12/14/20/28.

## Rust ↔ UI contract (exact names, the controller task wires them later)
In `app.slint` export:
```slint
export struct ServerItem { id: string, name: string, country: string /* ISO */, flag: image, summary: string,
    ping: int /* -2 untested, -1 failed, else ms */, has-quota: bool, quota-fraction: float, quota-text: string, selected: bool }
export struct SubscriptionItem { id: string, name: string, pinned: bool, collapsed: bool, updating: bool,
    has-traffic: bool, traffic-fraction: float, traffic-text: string, expire-text: string, announce: string,
    error: string, servers: [ServerItem] }
export enum ConnState { disconnected, connecting, connected, error }
export component AppWindow inherits Window {
    in property <[SubscriptionItem]> subscriptions;
    in property <[ServerItem]> local-servers;
    in property <string> selected-name; in property <image> selected-flag; in property <string> ping-result;
    in property <ConnState> state; in property <string> status-text; in property <string> speed-text;
    in-out property <bool> tun-mode; in-out property <string> search-text; in property <bool> dark: true;
    in-out property <int> page; // 0 servers, 1 add, 2 routing, 3 settings, 4 stats, 5 about
    callback toggle-connect(); callback select-server(string); callback open-server(string);
    callback ping-selected(); callback ping-all(); callback update-subscription(string); callback update-all();
    callback toggle-pin(string); callback toggle-collapse(string); callback subscription-settings(string);
    callback delete-server(string); callback delete-subscription(string); callback import-clipboard();
    callback sort-by-ping(); callback mode-changed(bool);
}
```
`src/flags.rs`: `pub fn flag(iso: &str) -> slint::Image` — flags embedded with `include_bytes!` via a
`build.rs`-generated match (generate `$OUT_DIR/flags.rs` listing every file in `assets/flags`), loaded with
`slint::Image::load_from_svg_data`, cached in a `thread_local!` HashMap; unknown -> UN.

`src/demo.rs` + `main.rs`: run with `--demo` (and by default until the controller exists) — fills the UI with
fake data that exercises everything: 2 subscriptions (one with quota 15/100 GB + expiry + announce, one unlimited,
one collapsed), servers in RU/DE/NL/FI/US with mixed pings, one with per-server quota; `toggle-connect` cycles
disconnected → connecting (1.5 s timer) → connected; search filtering done in Rust (re-filter model on
`search-text` change) or Slint — your choice, document it.

## Screenshot proof (required)
Slint has a software renderer: add a hidden CLI flag `--screenshot <out.png>` that renders the window
(960×600, demo data, connected state) offscreen with `slint::platform::software_renderer` (MinimalSoftwareWindow,
render into a `SharedPixelBuffer<Rgb8Pixel>`, save PNG — add the `png` crate only for this, or the `image` crate with
png feature only) and exits. Run it on ru1:
    scripts/ru1 sh -c 'cargo run -p flamix -- --screenshot /home/builder/flamix-shot.png'
then fetch it: `scp -i ~/.ssh/id_builder builder@BUILD_HOST:flamix-shot.png docs/design/screenshot-main.png`,
LOOK at it (open the PNG with your image-reading ability) and iterate until it clearly matches the mockup.
Commit the final `docs/design/screenshot-main.png` (also a `--light` variant `screenshot-main-light.png`).

## Done
    scripts/ru1 cargo fmt --all --check
    scripts/ru1 cargo clippy -p flamix --all-targets -- -D warnings
    scripts/ru1 cargo build -p flamix
Commit on `oc/T07`. Report: branch, commit hash, what differs from the mockup and why.

# T08 — app controller: wire the Slint UI to the core (connect, subscriptions, ping, import)

Read `AGENTS.md` first (build rule: `scripts/ru1`; work incrementally — this is a big task, do it in the order below,
committing after each step). Contracts: `crates/flamix-core` (model, link, subscription, deeplink, config, store, ping),
`crates/flamix-sys` (runner, sysproxy, elevation, autostart, stats), UI contract in `app/flamix/ui/app.slint`
(`AppWindow` properties/callbacks, `SubscriptionItem`, `ServerItem`, `ConnState`) and the demo in `app/flamix/src/demo.rs`.

## Setup
    git -C /srv/dev/projects/flamix-client worktree add /srv/dev/projects/flamix-client-wt/T08 -b oc/T08
    cd /srv/dev/projects/flamix-client-wt/T08

## Files you own
`app/flamix/src/**` (new modules below), `app/flamix/Cargo.toml` (deps), small additions to `app/flamix/ui/app.slint`
ONLY if a needed property/callback is missing (add, never rename; mention it in the report). Do not edit crates/*.
If a core/sys API is missing or buggy, work around it in the app and list it in the report (the lead files a task).

## Architecture
- `main.rs`: parse args (`--demo`, `--screenshot`, `--minimized`, a deep link / subscription URL as positional arg),
  single-instance (step 6), build a multi-thread tokio runtime on a background thread, create `AppWindow`, create the
  `Controller`, run the Slint event loop. `--demo` keeps today's demo behaviour (no core, fake data).
- `controller.rs`: `Controller { state: Arc<Mutex<AppState>>, data_dir, rt: tokio::runtime::Handle, ui: slint::Weak<AppWindow>,
  conn: Arc<Mutex<Option<Connection>>> }`. UI callbacks only enqueue work (`rt.spawn`); results go back to the UI with
  `slint::invoke_from_event_loop` / `Weak::upgrade_in_event_loop`. Never block the UI thread.
- `view.rs` (pure, unit-tested): `AppState` -> `Vec<SubscriptionItem>`, local `Vec<ServerItem>`, selected name/flag;
  pinned subscriptions first; search filter (case-insensitive substring on server name; subscriptions with no match hidden);
  sort-by-ping mode; human bytes (`15 GB`, `1.2 MB/s`, binary 1024 base, one decimal under 10), `traffic-text` "used / total" or
  "∞", `expire-text` "до dd.mm.yyyy", per-server quota text "осталось X", ping int encoding (-2 untested, -1 failed), flags via
  `flags::flag(iso)`. Banners: show `provider.info_banner.text` (else `announce`) in `announce`; `last_error` / HWID limit
  ("Достигнут лимит устройств") in `error`.
- `persist.rs`: load via `store::load(store::data_dir())`; save debounced (≤1 per second, on a background task) after
  every mutation and synchronously on exit.

## Steps
1. **Load & show**: real state into the UI (replace demo unless `--demo`); selection (`select-server`) persists
   `selected_server`; collapse/pin toggles persist.
2. **Import**: `import-clipboard` (crate `arboard`) and positional arg -> `deeplink::parse_import`:
   `Subscription{url,name}` -> new `Subscription` (id = random hex, name from arg or empty) then update it;
   `Links` -> `link::parse_links(text, "local")` -> `local_servers` (dedupe by id); `Routing(..)` -> apply to the subscription
   of the selected server (or keep in a toast "нет подписки"). Show a status text for success/failure.
3. **Subscriptions**: `update-subscription(id)` / `update-all` with `subscription::update(&mut sub, &FetchOptions{ proxy:
   Some(socks5h://127.0.0.1:<socks_port>) when connected, hwid: state.device_id, .. })` — clone the sub, update outside the
   lock, write back by id; `updating` flag in the UI; max 3 in parallel. On startup: update subs whose interval elapsed or
   `provider.update_always`; then every 10 min check intervals (`settings.auto_update_subscriptions`).
   `delete-subscription`, `delete-server` (local servers only; for subscription servers hide is not supported — ignore).
4. **Connect / disconnect** (`toggle-connect`): state `connecting` -> `config::plan(&server, &settings)` ->
   `runner::start(&plans, &data_dir.join("run"))` -> Proxy mode & `settings.set_system_proxy`: `sysproxy::set(ProxySpec{
   127.0.0.1, http_port, socks_port, bypass})` -> `connected`. Errors -> `error` state + status text (the core's log tail,
   first line). Disconnect: `sysproxy::clear()` first, then `Running::stop()`. While connected, a 1 s ticker: `is_alive`
   (dead -> clear proxy, `error` "ядро остановилось" + log tail), traffic via `stats::xray_traffic` /
   `stats::singbox_traffic` for the core that owns the proxy outbound -> `speed-text` "↑ x/s ↓ y/s", accumulate into the
   server's `local_traffic` and `daily_traffic[today]`. Selecting another server while connected = reconnect.
   `mode-changed(tun)`: update `settings.mode`; if connected, reconnect. TUN on Windows without admin: status text
   "Для TUN нужен запуск от администратора" and a callback `restart-as-admin` (add to app.slint if missing) ->
   `elevation::restart_elevated(args)`.
   **Crash safety**: write `<data_dir>/run/proxy-set` when the system proxy is set, remove after clear; on startup, if the file
   exists -> `sysproxy::clear()` (previous run died). Same on Ctrl+C / window close (`on_close_requested`): disconnect, save, exit.
5. **Ping**: `ping-all` -> `ping::tcp_ping_many` over all visible servers (concurrency 16, timeout 3 s) -> `last_ping_ms`;
   `ping-selected` -> when connected and `settings.ping_method == "url"`: `ping::url_ping(settings.ping_url, socks5h proxy)`,
   else TCP -> `ping-result` "· 42 ms" / "· нет ответа". `sort-by-ping` toggles the view sort.
6. **Single instance + deep links**: a second launch must hand its args to the running instance and exit: the first instance
   listens on `127.0.0.1:<random port>` and writes the port + a random token to `<data_dir>/instance`; a new process reads
   it, connects, sends `token\nargs-json\n`; the first instance processes the args like step 2 and brings the window to
   front (`window().show()`; set `minimized` false). Stale file (connect fails) -> become the first instance.
   `--minimized`: start hidden only if `settings.minimize_to_tray` (tray itself is a later task; until then just start minimized).
7. **Settings side effects** used now: `launch_at_login` -> `autostart::set_enabled` on change (no UI yet — expose nothing new).

## Tests (required)
`view.rs` mapping/format/filter/sort (many cases), import routing for each `Import` variant (pure function that
returns the state change), single-instance message encode/decode + token check, crash-safety marker logic (temp dir).
Manual check on the build server: `cargo run -p flamix -- --screenshot shots/real.png` with `FLAMIX_HOME` pointing to a temp dir
pre-filled with a `state.json` you generate (2 subscriptions, servers) — the screenshot must show the real data path
(not demo). Commit the generated test `state.json` under `app/flamix/tests/fixtures/`.

## Done
    scripts/ru1 cargo fmt --all --check
    scripts/ru1 cargo clippy --workspace --all-targets -- -D warnings
    scripts/ru1 cargo test -p flamix
Commit after every step on `oc/T08`. Report: branch, commits, what works end-to-end, every core/sys API gap you hit.

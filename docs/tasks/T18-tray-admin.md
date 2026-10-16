# T18 — system tray + "restart as administrator" button

Read `AGENTS.md` first (build rule: `scripts/ru1`; Windows check: `scripts/ru1 scripts/check-windows.sh`; work incrementally).

## Setup
    git -C /srv/dev/projects/flamix-client worktree add /srv/dev/projects/flamix-client-wt/T18 -b oc/T18
    cd /srv/dev/projects/flamix-client-wt/T18

## Files you own
new `app/flamix/src/tray.rs` (+ `src/tray/` submodules), `app/flamix/src/main.rs` (only: `mod tray;`, creating the tray,
close-to-tray behaviour), `app/flamix/src/controller.rs` (only: small `pub(crate)` wrappers the tray needs, e.g.
`toggle_connect_public`, `connection_state()`, `quit()`), `app/flamix/ui/components/connection-panel.slint`,
`app/flamix/ui/app.slint` (only wiring for the new connection-panel properties), `app/flamix/Cargo.toml` (deps).
Other agents edit `ui/pages/*` and `src/pages/*` — don't touch those.

## 1. Tray
- Crates: Windows/macOS `tray-icon` (+ its `muda` menu); Linux `ksni` (StatusNotifierItem over D-Bus, pure Rust — works on
  KDE, GNOME with the AppIndicator extension, XFCE, etc.; no GTK build dependency). Use `cfg(target_os)` to pick; keep
  a common `Tray` API: `Tray::new(handle) -> Option<Tray>` (None if unavailable — never crash), `set_connected(bool, server)`.
- Icon: from `app/flamix/assets/app-icon/` PNG (32/64 px), connected state = normal, disconnected = desaturated variant
  generated at runtime (convert to grey in code) — no new asset files needed.
- Menu (Russian): "Подключиться"/"Отключиться" (toggles), current server name (disabled item), "Режим: Proxy / TUN" (radio
  or two items), "Показать Flamix", "Выход". Left click / activate = show the window.
- Thread model: tray events arrive on the tray's own thread/loop; forward to the Slint loop with
  `slint::invoke_from_event_loop` and to the controller. On macOS `tray-icon` must be created on the main thread
  (do it before `ui.run()` from the event loop via `slint::invoke_from_event_loop` / `Timer::single_shot`).
- Close behaviour: when `settings.minimize_to_tray` and a tray exists, closing the window hides it
  (`window().on_close_requested` → `HideWindow`) instead of quitting; "Выход" in the tray disconnects (the existing shutdown
  path), saves and exits (`slint::quit_event_loop`). Without a tray the window close quits as today.
- `--minimized` start: keep the window hidden only if a tray exists.

## 2. Restart as administrator (TUN on Windows)
The controller already has `restart-as-admin` (callback in `app.slint`) → `elevation::restart_elevated`. Add to the
connection panel: when `status` says admin is needed (add an `in property <bool> needs-admin` set by the controller when
TUN fails for that reason), show a small accent button "Перезапустить от администратора" under the status text.

## Tests / checks
Unit-test the pure parts (menu model from state, grey icon conversion). `scripts/ru1 cargo clippy --workspace --all-targets -- -D warnings`,
`scripts/ru1 cargo test -p flamix`, `scripts/ru1 scripts/check-windows.sh` must be green. Linux: if `ksni` needs a D-Bus
session at runtime, tray creation must fail gracefully (None) on the headless build host — verify with
`cargo run -p flamix -- --demo --screenshot shots/x.png` still working.
Commit on `oc/T18`. Report: branch, commit, crates + versions, what you could not verify (no desktop on the build host).

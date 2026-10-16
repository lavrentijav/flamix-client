# T25 — exit IP: measure the real egress per server, show and group it

Read `AGENTS.md` first (build rule: `scripts/ru1`; work incrementally).
Owner's correction: an exit must be determined by **actually connecting through the server and looking
at the outgoing IP**, never inferred from the input address/domain. This task adds that measurement,
stores it, shows it on the server list, and groups servers that share an exit so failover can prefer
same-exit backups (the failover *UI* is a later task).

The model already has `AppState.exit_ips: BTreeMap<String, ExitIp>` with
`ExitIp { ip: String, checked: i64 }` (commit `d587621`) — do not change `model.rs`.

## Setup
    git -C /srv/dev/projects/flamix-client worktree add /srv/dev/projects/flamix-client-wt/T25 -b oc/T25
    cd /srv/dev/projects/flamix-client-wt/T25

## Files you own
`crates/flamix-core/src/ping.rs`, `app/flamix/src/controller.rs`, `app/flamix/src/view.rs`,
`app/flamix/ui/model.slint`, `app/flamix/ui/components/server-row.slint`,
`app/flamix/ui/components/servers-panel.slint`, `app/flamix/src/demo.rs` (add demo exit IPs so the
screenshot shows groups), `app/flamix/src/main.rs` (only if a panel callback must be wired). Do NOT
touch `crates/flamix-core/src/config/**`, `crates/flamix-core/src/model.rs`, the settings/routing pages,
or `app/flamix/ui/pages/**`.

## Core helper (`ping.rs`)
- `pub const IP_ECHO_URLS: &[&str]` = `["https://api.ipify.org", "https://ifconfig.me/ip", "https://icanhazip.com"]`.
- `pub async fn fetch_via_proxy(url: &str, proxy: &str, timeout: Duration) -> Option<String>` — HTTP GET
  through the local SOCKS proxy (`socks5h://127.0.0.1:<port>`), returning the response body as text
  (reuse the existing `reqwest`/proxy setup in `url_ping`). Never panics; `None` on any failure.
- `pub fn parse_ip(body: &str) -> Option<String>` — trim, take the first whitespace-separated token,
  accept only a plausible IPv4 or IPv6 (`std::net::IpAddr::from_str`); otherwise `None`. Unit tests
  (v4, v6, "with newline", junk, empty).
- `pub async fn probe_exit_ip(proxy: &str, timeout: Duration) -> Option<String>` — try `IP_ECHO_URLS`
  in order, return the first parsed IP.

## Controller (`controller.rs`)
1. **Live exit for the current connection.** After a successful `connect`, and then every 10 min while
   connected, fetch the exit IP through the running proxy (`127.0.0.1:<socks_port>`) on the runtime and
   store `exit_ips[server_id] = ExitIp { ip, checked: now }` via `mutate` (so it persists/renders).
   Cancel/stop it on disconnect (reuse the ticker epoch pattern). The proxy URL must use the connection's
   socks port.
2. **Batch probe (disconnected only).** A public method e.g. `probe_exit_ips(self: &Arc<Self>, ids: Vec<String>)`
   that: refuses (with a status line) if currently connected; otherwise sequentially for each id — build
   `config::plan(&server, &settings)` (proxy mode), `runner::start`, `ping::probe_exit_ip` through that
   plan's socks port, `Running::stop`, store the result in `exit_ips`, and push a progress line to the
   status text ("Проверка IP: 3/12 …"). Use a distinct/known socks port and stop the core in all paths
   (including error/timeout) so no core leaks. Run it on the runtime; keep the UI responsive.
   Make the loop testable by extracting the pure parts (progress text, which ids to probe).
3. A pure helper `exit_groups(state) -> BTreeMap<String, Vec<String>>` (ip -> sorted server ids) for the
   UI, with unit tests. Servers with no measured IP are excluded.

## UI
- `model.slint` `ServerItem`: add `in property <string> exit-ip;` and `in property <int> exit-group;`
  (0 = none, N = size of its exit group when >1). Fill them in `view::server_item` from
  `state.exit_ips` + `exit_groups`.
- `server-row.slint`: show the exit IP in muted small text under the server name/summary; when
  `exit-group > 1`, show a small badge (e.g. "IP ×N" or a dot with a tooltip) so the user sees which
  servers share an exit. Keep the row layout clean and touch-free (desktop).
- `servers-panel.slint`: add a header button "Проверить выходные IP" → a new callback `check-exit-ips()`;
  wire it in `main.rs`/controller to `probe_exit_ips(all ids)` (when connected, it should just show the
  "disconnect first" status). Keep the existing buttons' style.
- Do not start a probe automatically on startup (too heavy); only on the button, plus the passive live
  probe of the connected server.

## Done
    scripts/ru1 cargo fmt --all --check
    scripts/ru1 cargo clippy --workspace --all-targets -- -D warnings
    scripts/ru1 cargo test --workspace
    scripts/ru1 scripts/check-windows.sh
    FLAMIX_FETCH=shots scripts/ru1 sh -c 'cargo run -p flamix -- --demo --screenshot shots/main.png --page 0'
Regenerate `docs/design/screenshot-main.png` (+ light) showing at least two servers with exit IPs and a
shared-exit badge in `--demo`. Commit on `oc/T25`. Report: branch, commit, how the passivity/batching
works, test counts, and what you could not verify. Do not push, do not merge.

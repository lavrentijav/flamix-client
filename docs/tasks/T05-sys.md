# T05 — OS integration: system proxy, core runner, elevation, autostart

Read `AGENTS.md` first (build rule: everything via `scripts/ru1`). Stubs with documented behaviour:
`crates/flamix-sys/src/{sysproxy,runner,elevation,autostart}.rs`, contract types in
`crates/flamix-core/src/config/mod.rs` (`CoreKind`, `CorePlan`).

## Setup
    git -C /srv/dev/projects/flamix-client worktree add /srv/dev/projects/flamix-client-wt/T05 -b oc/T05
    cd /srv/dev/projects/flamix-client-wt/T05

## Files you own
`crates/flamix-sys/**` only. Keep public signatures; you may add functions/types.

## Implement (all three desktop OSes, `#[cfg(target_os = ...)]` modules per file)
- **sysproxy** as documented. Windows: `winreg` + `InternetSetOptionW(INTERNET_OPTION_SETTINGS_CHANGED)` and
  `INTERNET_OPTION_REFRESH` (`unsafe` is allowed in this crate only for these FFI calls, each with a `// SAFETY:` comment).
  ProxyServer value `http=127.0.0.1:P;https=127.0.0.1:P;socks=127.0.0.1:S`. macOS: list services with
  `networksetup -listallnetworkservices` (skip the header line and `*`-disabled ones), set web/secureweb/socks proxy
  + bypass domains, `clear` turns them off. Linux: GNOME (`gsettings set org.gnome.system.proxy mode 'manual'`, http/https/socks
  host+port, ignore-hosts as a GVariant string array) and KDE (`kwriteconfig6` or `kwriteconfig5` on `kioslaverc`,
  `ProxyType 1`, `httpProxy "http://127.0.0.1 P"`, …, then `dbus-send --type=signal /KIO/Scheduler org.kde.KIO.Scheduler.reparseSlaveConfiguration string:''`);
  run whatever is available, error only if every attempted backend failed. Bypass list conversion per OS
  (`<local>` is Windows-only; on GNOME map `127.*` -> `127.0.0.0/8`, etc.).
- **runner**: `find_core` per doc (`.exe` suffix on Windows; must be a file and executable). `start`: write configs,
  spawn with `tokio::process::Command` (`kill_on_drop(true)`, stdout+stderr piped, read lines into a ring buffer of 500
  lines shared via `Arc<Mutex<VecDeque<String>>>`), Windows: `CREATE_NO_WINDOW` creation flag. Xray: `xray run -c <file>`;
  sing-box: `sing-box run -c <file> -D <work_dir>` (working dir for rule-set cache). Wait 1.5 s and check
  `try_wait`; on early exit stop the others and return `Error::CoreExited(last 20 log lines)`.
  Elevated plans: Linux — if `elevation::is_elevated()` or the binary has `cap_net_admin` (check via `getcap` output, best effort) run directly, else
  `pkexec <bin> run -c <file> -D <dir>`; macOS — run via `osascript ... with administrator privileges` a shell
  command that starts the core in background writing its PID to `<work_dir>/sing-box.pid` and its log to `<work_dir>/sing-box.log`
  (tail the log file into the ring buffer); stopping = `osascript` admin `kill <pid>`; Windows — if not elevated return
  `Error::Command("TUN mode needs Flamix to run as administrator")` (the UI offers "restart as admin", see below).
  `stop`: graceful (SIGTERM on unix via `kill` command or `nix`-free `libc`-free approach: `Command::new("kill")`), wait 3 s, then kill.
- **elevation**: `is_elevated()` — unix: `geteuid()==0` read without libc: parse `/proc/self/status` `Uid:` on Linux,
  `id -u` command on macOS; Windows: `IsUserAnAdmin` (Win32_UI_Shell) or token elevation. Add
  `pub fn restart_elevated(args: &[String]) -> Result<()>` — Windows: `ShellExecuteW` "runas" on the current exe;
  other OSes `Err(Unsupported)`.
- **autostart** as documented (exe path from `std::env::current_exe()`, arg `--minimized`). Linux `.desktop` with
  `X-GNOME-Autostart-enabled=true`. macOS LaunchAgent plist with `RunAtLoad`.
- Add `pub mod stats;` — traffic counters: `pub async fn xray_traffic(xray_bin: &Path, api_port: u16) -> Result<(u64, u64)>`
  (runs `xray api statsquery --server=127.0.0.1:<port> -pattern "outbound>>>proxy>>>traffic" -reset` and sums
  `uplink`/`downlink` values from its JSON output — returns the delta since the last call) and
  `pub async fn singbox_traffic(api_port: u16) -> Result<(u64, u64)>` (GET `http://127.0.0.1:<port>/connections`
  of the clash API, return `uploadTotal`/`downloadTotal`; caller computes deltas). Use a tiny raw HTTP/1.1 request over
  `tokio::net::TcpStream` instead of adding reqwest here.

## Tests
Unit-test pure parts: bypass conversions per OS (as pure functions, compiled on all OSes), Windows ProxyServer string,
macOS service list parsing (sample output), statsquery JSON parsing, clash `/connections` parsing, `find_core` with a temp
dir + `FLAMIX_CORE_DIR`. Integration test (Linux, on ru1): start a fake core = a shell script that echoes and sleeps;
check `log_tail`, `is_alive`, `stop`; and a script that exits immediately -> `CoreExited`.

## Cross-OS check
Add the targets on ru1 (`scripts/ru1 rustup target add x86_64-pc-windows-gnu aarch64-apple-darwin`) and run
`scripts/ru1 cargo check -p flamix-sys --target x86_64-pc-windows-gnu` and `--target aarch64-apple-darwin`.
If a dependency's C build script (e.g. `ring`) blocks the check for a target, report it — do not hack around it.

## Done
    scripts/ru1 cargo fmt --all --check
    scripts/ru1 cargo clippy -p flamix-sys --all-targets -- -D warnings
    scripts/ru1 cargo test -p flamix-sys
Commit on `oc/T05`. Report: branch, commit, tests, cross-check results.

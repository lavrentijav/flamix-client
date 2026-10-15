# flamix-client — rules for agents

Cross-platform VPN client in Rust (desktop: Windows, macOS, Linux; mobile later).
UI: Slint (Fluent / WinUI3 look). Proxy cores: external Xray-core and sing-box (like v2rayN).
General environment rules: `/srv/dev/AGENTS.md`, `~/.claude/CLAUDE.md` (mandatory).

## Layout
- `crates/flamix-core` — platform independent logic (model, links, subscriptions, configs, store, ping). No OS calls except files/network.
- `crates/flamix-sys` — OS integration (system proxy, core processes, elevation, autostart).
- `app/flamix` — Slint desktop app (`ui/*.slint`, `src/*.rs`).
- `docs/` — specs (`subscription-headers.md`, `architecture.md`), `docs/tasks/` — task briefs.
- `scripts/` — build/packaging scripts. `scripts/ru1` runs any command on the build server.

## BUILD RULE (strict)
ALL compilation, tests, clippy and packaging run on the **ru1** server, never locally:

    scripts/ru1 cargo test -p flamix-core
    scripts/ru1 cargo clippy --workspace --all-targets -- -D warnings
    scripts/ru1 cargo fmt --all --check

`scripts/ru1` syncs the current checkout (any worktree) and runs there under a lock — builds of
parallel agents queue, so be patient; do not run several cargo commands at once.
`cargo fmt` (no `--check`) is fine locally if rustfmt exists, otherwise run it via `scripts/ru1`
and then apply the same formatting by hand — the remote copy is not synced back.

## Code rules
- `model.rs` is the shared contract: do not change existing types/fields without a task saying so.
  Adding a `#[serde(default)]` field is OK when your task needs it; mention it in the report.
- Only edit the files your task owns. Public signatures given in stubs are the contract.
- No `unwrap()`/`expect()` on data from the network/user; return `Result`. Tests may unwrap.
- Every parser gets unit tests with real-world samples. `cargo clippy -D warnings` clean, `rustfmt`.
- No new heavy dependencies without need; prefer workspace deps (root `Cargo.toml`).
- Comments in English, UI strings in Russian + English (see `app/flamix/src/i18n.rs`).

## Git
- Work in your own worktree/branch: `git -C /srv/dev/projects/flamix-client worktree add /srv/dev/projects/flamix-client-wt/<id> -b oc/<id>`.
- Local commits with meaningful messages. Never push, never merge, never touch other branches.

## How to work (important for opencode agents)
- Reasoning per turn is capped (~32k tokens); a turn that only thinks is cut off with NO output.
  Never plan a whole module in your head: write one file / one function group per step, test, continue.
- Run `scripts/ru1` checks after every couple of files, not only at the end.
- You cannot view images; visual review is done by the lead.

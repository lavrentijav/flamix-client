# T23 — failover, stage 2: wire it into the controller + per-node traffic counters

Read `AGENTS.md` first (build rule: `scripts/ru1`; work incrementally, one file at a time).
`docs/failover.md` is the design; stage 1 (T21, merged) added the `Failover` model and
`config::plan_cluster`. This stage makes failover actually take effect when it is enabled, and fixes the
traffic counter so it no longer reads an outbound that does not exist under failover. **No UI here**
(a separate task adds the backup-list editor); failover is driven by `settings.failover`, which tests set
directly. TUN mode keeps using `plan`/`plan_with` (failover is Proxy-only, per T21).

## Setup
    git -C /srv/dev/projects/flamix-client worktree add /srv/dev/projects/flamix-client-wt/T23 -b oc/T23
    cd /srv/dev/projects/flamix-client-wt/T23

## Files you own
`app/flamix/src/controller.rs`, `crates/flamix-sys/src/stats.rs`. Nothing else.

## Do
1. **Build the node list in `connect`.** After resolving `server`/`settings`/`provider`/`overrides`, if
   `settings.failover.enabled` and `settings.mode == ConnectionMode::Proxy`, resolve
   `settings.failover.backups` (server ids) through `state.server(id)`, in order, skipping ids that do not
   exist and skipping the primary's own id (dedupe). The primary is `server`; the slice `[primary, backups…]`.
   Then call `config::plan_cluster(&nodes, &settings, &ctx)` instead of `config::plan_with`. When failover is
   off (or TUN), keep `config::plan_with` exactly as today. Add a tiny pure helper (unit-tested) that builds
   this slice from `&AppState` and the primary id, so the selection logic is testable without a live core.
2. **Counters (Proxy + failover only).** `stats::xray_traffic` currently runs
   `xray api statsquery -pattern "outbound>>>proxy>>>traffic"`, but under failover the Xray config has no
   `proxy` outbound (only `node-0…node-N`; `proxy` is a balancer tag without counters), so traffic reads 0.
   Change `xray_traffic` to take the pattern (or an enum): keep the current pattern for the single-server
   case and use `outbound>>>node-` (prefix match; `parse_xray_stats` already sums every matching name) when
   the connection is failover. Store on `Connection` whether failover stats are in use (set it at connect
   time from the same condition as step 1) and pick the pattern in `sample_traffic`. `direct`/`block`/`api`
   counters must not be counted. Add unit tests for the pattern choice.
3. **Reconnect on failover change.** `reconnect_if_connected` is already used by the pages; make sure a change
   to `settings.failover` (enable/disable, backups) triggers a reconnect the way other core-affecting
   settings do. `controller.rs` owns no settings-diff today (the settings page does), so if there is no hook
   here, at minimum document that the settings/routing page must call `reconnect_if_connected` on failover
   edits, and make the connect path re-read `settings.failover` fresh (it already re-reads `state.settings`).
4. **Status text.** When connected with failover on, the status line should still be "Подключено"; do not
   invent new strings. (A later task adds per-node status.)

## Constraints
- Never change behaviour when `settings.failover.enabled == false` (must be byte-for-byte the old plan and the
  old stats pattern) or in TUN mode.
- No `unwrap`/`expect` on external data; return `Result`/fallbacks. Comments in English. `-D warnings` clean.
- Do not touch `model.rs` or `config/**` (stage 1 owns them) unless a signature is missing; if it is, stop and
  report instead of editing.

## Done
    scripts/ru1 cargo fmt --all --check
    scripts/ru1 cargo clippy --workspace --all-targets -- -D warnings
    scripts/ru1 cargo test --workspace
Commit on `oc/T23`. Report: branch, commit, the stats behaviour with/without failover, test counts, and
anything unfinished. Do not push, do not merge.

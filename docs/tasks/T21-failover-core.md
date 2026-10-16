# T21 — failover, stage 1: core model + balancer configs (no gate, no UI)

Read `AGENTS.md` first (build rule: `scripts/ru1`; work incrementally, one file at a time).
Design: `docs/failover.md` (this task = its **T12**, core only). Use the **core-level fallback**
(no `flamix-gate` yet): one core process with an outbound per node and an auto-selecting balancer.
This stage is **proxy-mode only**; TUN mode stays exactly as today, and the app/controller wiring is a
separate task — here you only add the model + the config generators, verified by unit/real-core tests.

## Setup
    git -C /srv/dev/projects/flamix-client worktree add /srv/dev/projects/flamix-client-wt/T21 -b oc/T21
    cd /srv/dev/projects/flamix-client-wt/T21

## Files you own
`crates/flamix-core/src/model.rs`, `crates/flamix-core/src/config/**` and their tests. Nothing else
(no `controller.rs`, no app). Adding a field to `Settings` may break struct literals in tests elsewhere in
flamix-core — fix only the ones the compiler reports, preferring `..Default::default()`.

## Model (`model.rs`)
Add, with `#[serde(default)]` behaviour (never break an old `state.json`):

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum FailoverMode { #[default] Failover, LoadBalance }

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Failover {
    pub enabled: bool,            // false
    pub mode: FailoverMode,       // Failover
    pub backups: Vec<String>,     // server ids, ordered (primary is the selected server)
    pub probe_interval_ms: u32,   // 15000
    pub probe_url: String,        // empty => use settings.ping_url
    pub tolerance_ms: u32,        // 50
}
impl Default for Failover { fn default() -> Self { /* the values above */ } }
```
Add `pub failover: Failover` to `Settings` and set it in `Settings::default()`. Keep types/fields that
already exist unchanged.

## Config (`config/mod.rs`)
Keep `plan(server, settings)` and `plan_with(server, settings, ctx)` and add:

```rust
pub fn plan_cluster(nodes: &[Server], settings: &Settings, ctx: &ConnectContext) -> Vec<CorePlan>
```

`nodes[0]` is the primary; the rest are ordered backups. `plan`/`plan_with` must delegate with a
one-element slice so nothing regresses. Failover is applied **only** when
`settings.failover.enabled && nodes.len() > 1 && settings.mode == ConnectionMode::Proxy`; otherwise the
generated config must be byte-for-byte what it is today.

## Xray (`config/xray.rs`)
When failover is on, build the remote path from N outbounds and a balancer (routing/API behaviour must not change):
- Outbounds: one per node with tags `node-0`, `node-1`, … (primary = `node-0`); keep `direct`/`block`.
- `routing.balancers`: `[{"tag":"proxy","selector":["node-0",…],"strategy":{"type":…}}]` — `leastPing` for
  `Failover`, `roundRobin` for `LoadBalance`.
- Top-level `observatory`: `{"subjectSelector":["node-0",…],"probeUrl":<failover.probe_url or settings.ping_url>,
  "probeInterval":"…","enableConcurrency":true}` (`leastPing` needs it; harmless for `roundRobin`).
- Every existing routing rule that pointed at the outbound tag `proxy` (custom rules, provider final,
  preset catch-all) must keep working: with the balancer the tag `proxy` now names the balancer, so the
  routing generator itself should be unchanged — verify the single-node path still emits the outbound tag
  `proxy` as today.

## sing-box (`config/singbox.rs`)
When failover is on, tags `node-0`… for the real outbounds and add a `urltest` outbound with `"tag":"proxy"`:
`{"type":"urltest","tag":"proxy","outbounds":["node-0",…],"url":<probe url>,"interval":"…","tolerance":<tolerance_ms>}`.
`route.final` and DNS/server `detour` references keep pointing at `proxy`. When off, emit the single
outbound tagged `proxy` exactly as now.

## Constraints
- No `unwrap`/`expect` on external data; `config` returns `Value`s. Comments in English. `-D warnings` clean.
- Provider routing (`ConnectContext`) is applied to `node-0` only (document this; per-node provider context is a later task).
- Real-core validation is required: extend the existing real-binary tests so a multi-node Proxy config for
  both cores (mix at least two transports, e.g. vless+reality and trojan+ws) is accepted by the cores,
  `Failover` and `LoadBalance`, and the disabled path is unchanged.

## Done
    scripts/ru1 cargo fmt --all --check
    scripts/ru1 cargo clippy --workspace --all-targets -- -D warnings
    scripts/ru1 sh -c 'FLAMIX_XRAY_BIN=$HOME/flamix-tools/cores/xray FLAMIX_SINGBOX_BIN=$HOME/flamix-tools/cores/sing-box cargo test --workspace -- --include-ignored'
Commit on `oc/T21`. Report: branch, commit, the exact JSON you emit for the balancer/urltest, test counts,
and anything the cores rejected. Do not push, do not merge.

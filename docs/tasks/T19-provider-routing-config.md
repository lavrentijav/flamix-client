# T19 — apply provider routing and Happ app-overrides in the core configs

Read `AGENTS.md` first (build rule: `scripts/ru1`; work incrementally). Model: `ProviderExtras`, `ProviderRouting`,
`SubscriptionOptions` in `crates/flamix-core/src/model.rs`; parsing is done (`crates/flamix-core/src/provider_routing.rs`,
`docs/subscription-headers.md` "Compatibility"); configs in `crates/flamix-core/src/config/`.

## Setup
    git -C /srv/dev/projects/flamix-client worktree add /srv/dev/projects/flamix-client-wt/T19 -b oc/T19
    cd /srv/dev/projects/flamix-client-wt/T19

## Files you own
`crates/flamix-core/src/config/**`. Plus ONE call-site change in `app/flamix/src/controller.rs` (`connect`: pass the
provider routing of the selected server's subscription — see below); keep that diff minimal, another agent edits that file.

## Do
1. API: add `pub struct ConnectContext<'a> { pub provider: Option<&'a ProviderRouting>, pub overrides: Option<&'a BTreeMap<String, String>> }`
   (Default = none) and `pub fn plan_with(server, settings, ctx: &ConnectContext) -> Vec<CorePlan>`; `plan()` stays and calls
   `plan_with` with the default context. Thread `ctx` into `xray::full_config` / `singbox::full_config` (new `*_with` fns or
   extra param — keep old signatures working for existing tests).
2. Provider routing (when `Some` and `enabled`): its rules go BEFORE the user's custom rules and REPLACE the user's preset
   (provider decides); `global_proxy == false` → final/unmatched traffic goes `direct` (else proxy); `domain_strategy` overrides
   settings; `dns` overrides remote/direct DNS; `dns_hosts` → Xray `dns.hosts` / sing-box DNS `hosts`-type server or
   `predefined` rules (pick what sing-box 1.14 accepts — validate with the real binary); `fake_dns` → fake-ip in TUN;
   custom `geoip_url`/`geosite_url` → Xray can't download: ignore with a `tracing::warn!` and note it; sing-box: use them only
   if they are `.srs` rule-set URLs, else ignore.
3. Overrides (Happ app-management keys, lower-case): `socks-auth-mode`=`password` + `socks-auth-user`/`-password` → auth on the
   local SOCKS inbound (Xray `accounts`, sing-box mixed `users`); same for `http-auth-*` on the HTTP inbound. Other keys ignored.
4. Controller call site: in `connect`, find the subscription that owns the server; if
   `!sub.options.ignore_provider_routing` pass `sub.provider.routing.as_ref()` and `Some(&sub.provider.app_overrides)` via
   `ConnectContext` to `plan_with`. When local auth is set, the controller's system proxy keeps working for HTTP clients only
   if auth is off — if auth is on, skip setting the system proxy and set a status text "Системный прокси выключен: провайдер
   включил авторизацию". Local servers: default context.
5. Tests: unit tests for both cores (provider rules order, global_proxy false → final direct, dns override, hosts, auth), and
   extend the real-binary validation (`FLAMIX_XRAY_BIN` / `FLAMIX_SINGBOX_BIN` on the build server:
   `/home/builder/flamix-tools/cores/{xray,sing-box}`) with provider-routing configs; deprecation WARN = failure.

## Done
    scripts/ru1 cargo fmt --all --check
    scripts/ru1 cargo clippy --workspace --all-targets -- -D warnings
    scripts/ru1 sh -c 'FLAMIX_XRAY_BIN=$HOME/flamix-tools/cores/xray FLAMIX_SINGBOX_BIN=$HOME/flamix-tools/cores/sing-box cargo test --workspace -- --include-ignored'
Commit on `oc/T19`. Report: branch, commit, test counts, anything the cores rejected.

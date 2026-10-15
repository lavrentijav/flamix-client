# T15 — Happ / v2RayTun / Remnawave compatibility: headers, deep links, provider routing (parsing)

Read `AGENTS.md` first (build rule: `scripts/ru1`; work incrementally). Facts with sources:
`docs/research/happ.md`, `docs/research/v2raytun.md`, `docs/research/gaps.md` ("Compatibility we should accept"),
`docs/research/hiddify.md`, `docs/research/v2rayng.md`. Contract: `crates/flamix-core/src/model.rs` — new types
`ProviderExtras`, `ProviderRouting`, `InfoBanner`, `HwidStatus`, `SubscriptionOptions` and the fields
`Subscription::provider`, `Subscription::options` (added by the lead; their doc comments are the spec).

## Setup
    git -C /srv/dev/projects/flamix-client worktree add /srv/dev/projects/flamix-client-wt/T15 -b oc/T15
    cd /srv/dev/projects/flamix-client-wt/T15

## Files you own
`crates/flamix-core/src/subscription/**`, `crates/flamix-core/src/deeplink.rs`, new `crates/flamix-core/src/provider_routing.rs`
(add `pub mod provider_routing;` to lib.rs), `docs/subscription-headers.md` (add a "Compatibility" section).
First fix the build: struct literals of `Subscription` in tests (fetch.rs, deeplink.rs) need
`provider: Default::default(), options: Default::default()` (or `..Default::default()`-style helper).
Do NOT edit config/* (another agent) — applying provider routing in the core configs is a later task.

## Implement
1. **Meta** (`SubscriptionMeta` gets the new fields; header OR `#key: value` body line, header wins):
   `fallback-url` (Happ) = alias of `profile-backup-urls` (merge); `providerid` (header, body `#providerid X`, URL
   fragment `#?providerid=X` of the subscription URL); `routing` (value is `happ://routing/...` link, or base64 Xray routing
   JSON for v2RayTun, or base64 Happ profile JSON) -> `ProviderRouting`; `routing-enable`; `announce-url`;
   `sub-info-text/color/button-text/button-link` -> `InfoBanner`; `sub-expire` / `sub-expire-button-link`; `update-always`;
   `network-filter`; `subscription-refill-date` (unix seconds or RFC 3339 / `YYYY-MM-DD`); response headers `x-hwid-limit`,
   `x-hwid-active`, `x-hwid-max-devices-reached`, `x-hwid-not-supported` -> `HwidStatus`; every other Happ app-management key
   from docs/research/happ.md -> `app_overrides` (lower-case key -> raw value). A raw `happ://routing/...` line in the body is
   also accepted (and removed before link parsing).
2. **`provider_routing.rs`**: `pub fn from_happ_profile(json: &serde_json::Value) -> Result<ProviderRouting>` (keys per
   docs/research/happ.md: Name, GlobalProxy, RemoteDNS*/DomesticDNS*, Geoipurl, Geositeurl, DnsHosts, DirectSites, DirectIp,
   ProxySites, ProxyIp, BlockSites, BlockIp, DomainStrategy, FakeDNS) — rule order Block, Direct, Proxy;
   `pub fn from_xray_routing(json) -> Result<ProviderRouting>` (v2RayTun: `rules[]` with `domain`/`ip`/`port`/`network`/
   `outboundTag` direct|proxy|block; unknown outboundTags -> proxy; balancers ignored with a tracing warn);
   `pub fn from_happ_link(link: &str) -> Result<RoutingLinkAction>` where
   `enum RoutingLinkAction { Add(ProviderRouting), AddAndActivate(ProviderRouting), Off }` for
   `happ://routing/{add,onadd}/<base64>` and `happ://routing/off`. Tolerate std/url-safe base64, BOM, case of keys.
3. **Fetch**: send `options.custom_headers` (they override defaults except Host); after parsing apply `options.name_filter`
   (regex — add the `regex` crate to flamix-core, workspace dep) and `provider.network_filter` (keep servers whose
   `transport.network` / protocol name matches any comma-separated token; empty = no filter); store all extras into
   `sub.provider`. Keep `enabled` of an existing provider routing if the user disabled it, unless `routing-enable` says otherwise.
   On HTTP error responses still read the `x-hwid-*` headers (so the UI can say "device limit reached").
4. **Deep links** (`deeplink.rs`): add `Import::Routing(RoutingLinkAction)`; accept `happ://routing/...`;
   `v2rayng://install-config?url=` (fragment = name), `hiddify://install-config?url=..&name=..#name` and path form
   `hiddify://<url>#<name>`, `v2ray://`, `v2rayn://`, `clashmeta://` with `?url=`/`name=`; fragment as name for v2rayng.
   Encrypted `happ://crypt*/` and `v2raytun://crypt/` -> `None` plus a `tracing::info!` (proprietary keys, unsupported).
5. **Docs**: in `docs/subscription-headers.md` add "Compatibility (Happ / v2RayTun / Remnawave)" listing every accepted key.

## Tests
Real examples from the research docs for each header; Happ profile JSON sample -> rules; v2RayTun base64 routing sample;
routing deep links add/onadd/off; name_filter + network_filter; custom headers reach a local TcpListener server; hwid headers
on a 403 response; every new deep link form.

## Done
    scripts/ru1 cargo fmt --all --check
    scripts/ru1 cargo clippy -p flamix-core --all-targets -- -D warnings
    scripts/ru1 cargo test -p flamix-core
(link-parser tests may still be ignored/failing if T01 isn't merged — say which.) Commit on `oc/T15`, report.

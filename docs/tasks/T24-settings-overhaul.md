# T24 — settings overhaul: wire the new model into the cores + rebuild the Settings page

Read `AGENTS.md` first (build rule: `scripts/ru1`; work incrementally, one file at a time).
The owner's feedback: "settings are far too few, v2rayN is the model — give everyone a good basic
set and let those who want go deeper". The model already has the new fields (commit `d587621`);
this task **wires them into the config generators and renders them in the Settings page** as two
tiers: **Основные** (always visible) and **Дополнительно** (collapsed/expanded).

## Setup
    git -C /srv/dev/projects/flamix-client worktree add /srv/dev/projects/flamix-client-wt/T24 -b oc/T24
    cd /srv/dev/projects/flamix-client-wt/T24

## Files you own
`crates/flamix-core/src/config/**`, `crates/flamix-core/src/model.rs` (only if a type must change —
prefer not), `app/flamix/ui/pages/settings-page.slint`, `app/flamix/ui/pages/settings-card.slint`,
`app/flamix/src/pages/settings.rs`. Do NOT touch `controller.rs`, other pages, `view.rs`, `app.slint`
or `main.rs`.

## New `Settings`/`Dns` fields to wire (already in the model)
Local inbound: `udp_enabled`, `sniffing`, `dest_override: Vec<String>`, `route_only`,
`local_auth_user`, `local_auth_pass`.
TUN: `tun_auto_route`, `tun_strict_route`, `tun_icmp` (rule|direct|unreachable|drop|reply),
`tun_route_exclude: Vec<String>` (CIDRs).
DNS (`settings.dns`): `fake_ip_range`, `bootstrap`, `block_svcb`, `parallel`, `serve_stale`,
`hosts: BTreeMap<String,String>`.
Other: `mux_concurrency`, `core_override` ("auto"|"xray"|"sing-box").

## Wiring rules (config)
1. **Local inbound (both cores).** `udp_enabled` controls UDP on the local SOCKS/HTTP inbound (Xray
   `settings.udp`; sing-box mixed). When `sniffing` is on, emit `sniffing.enabled` with
   `destOverride = dest_override` and `routeOnly = route_only` (Xray `inbounds[].sniffing`; sing-box
   `sniff`/`sniff_override_destination`/`domain_strategy` equivalents — validate with the real binary).
   `local_auth_user`/`pass` (when both non-empty) become local inbound auth; keep the existing
   provider app-overrides (`socks-auth-*`/`http-auth-*`) override path working and let the provider
   value win over the user setting; document the precedence.
2. **TUN (sing-box).** Map `tun_auto_route`, `tun_strict_route`, `tun_route_exclude`
   (`route_exclude_address`), and `tun_icmp` to the sing-box tun inbound. Keep current behaviour with
   the defaults (`auto_route=true`, `strict_route=false`, `icmp=reply`).
3. **DNS.** `dns.fake_ip_range` → fake-ip range where the core supports it; `dns.bootstrap` →
   bootstrap server; `dns.block_svcb` (sing-box), `dns.parallel`/`dns.serve_stale` (Xray) → emit only
   where the core accepts them, else ignore with a one-line `tracing::debug!` (never fail the config);
   `dns.hosts` → Xray `dns.hosts` / sing-box DNS `hosts` (use the mechanism that the real core
   accepts; there is already provider `dns_hosts` behaviour to mirror).
4. **Mux.** `mux_concurrency` → Xray mux `concurrency` (only when `mux` is on). sing-box mux if
   supported by 1.14, else ignore.
5. **Core override.** `core_override` in `config::plan*`: "auto" keeps today's choice; "xray" forces
   Xray when the server is Xray-capable (else fall back to auto and log), "sing-box" forces sing-box.
   Do not break the failover/TUN paths.
6. **Back-compat.** With the default values above the generated configs must stay equivalent to today.
   Defaults: udp on, sniffing on, dest_override ["http","tls"], auto_route true, strict_route false,
   icmp reply, parallel true, serve_stale false, fake_ip_range "198.18.0.0/15", mux_concurrency 8,
   core_override "auto".

## UI (Settings page, two tiers)
- Rebuild `settings-page.slint` into clear sections. **Основные** (top, always open): Подключение
  (mode, socks/http ports, allow LAN, set system proxy, UDP, sniffing), TUN (stack/MTU/IPv6,
  auto-route, strict-route), DNS (remote, direct, fake-ip), Пинг (method, url), Приложение
  (launch at login, connect on launch, minimize to tray, auto-update subs, theme, language, log level).
- **Дополнительно** (a collapsible group, collapsed by default; a small "Показать дополнительные
  настройки" ToggleButton/Switch at the bottom): dest-override checkboxes (http/tls/quic/fakedns),
  route-only, local auth user/pass, ICMP routing combo, route-exclude CIDRs (multi-line), DNS:
  fake-ip range, bootstrap, block SVCB, parallel query, serve stale, hosts (multi-line `domain ip`),
  mux concurrency SpinBox, core override ComboBox.
- Keep the existing card style (`settings-card.slint`), 8px radius, Russian labels, debounced save
  (500 ms) via the existing `save()`/`mutate` path, inline validation, reset button.
- Multi-line list fields (route-exclude CIDRs, hosts, dest-override) must round-trip through helpers
  with unit tests (parse/format), like the existing `bypass_from_text`/`bypass_to_text`.
- Wire every new bridge property in `write_bridge`/`read_bridge`/`apply_owned_in_place`/`merge_owned`
  and extend `core_affecting_changed` so UDP/sniffing/TUN/DNS/mux/core changes trigger
  `reconnect_if_connected`. Also add `failover` to `core_affecting_changed` (was documented as a gap).

## Done
    scripts/ru1 cargo fmt --all --check
    scripts/ru1 cargo clippy --workspace --all-targets -- -D warnings
    scripts/ru1 sh -c 'FLAMIX_XRAY_BIN=$HOME/flamix-tools/cores/xray FLAMIX_SINGBOX_BIN=$HOME/flamix-tools/cores/sing-box cargo test --workspace -- --include-ignored'
    scripts/ru1 scripts/check-windows.sh
    FLAMIX_FETCH=shots scripts/ru1 sh -c 'cargo run -p flamix -- --demo --screenshot shots/settings.png --page 3'
Regenerate `docs/design/screenshot-settings.png` and `-settings-light.png` (also an "advanced open"
shot if the task makes that easy) and commit them. Unit tests for every new helper and for each wired
config knob (real-core validation for TUN/DNS changes). Commit on `oc/T24`. Report: branch, commit,
the exact JSON the new knobs emit, test counts, anything a core rejected, and any field you could not
wire. Do not push, do not merge.

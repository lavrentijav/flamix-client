# Gaps — features other clients have that Flamix's current plan lacks

Compared against `docs/subscription-headers.md`, `crates/flamix-core/src/model.rs` and
`docs/tasks/*.md`. "Value" = user/provider value; complexity S/M/L; place = the Flamix crate the work
belongs to (core / sys / app / packaging). Sorted by value within each band.

## High value

1. **Policy groups / delay-based auto-selection / failover** — a virtual "server" that picks the
   best/lowest-latency member or auto-fails-over. Apps: v2rayN (4 Xray modes + sing-box lowest
   latency; failover = 5 s tolerance), Hiddify (delay-based selection), also common in Remnawave
   setups. Value **high** (core UX). Complexity **L**. Place **core** (model + `config` generator)
   + **app** (list UI). Needs a new `Server`-like entity or a `PolicyGroup` in the model.
2. **`routing` subscription header / forced routing profile** — provider ships a routing profile that
   overrides app routing. Apps: **Happ** (`routing`, `happ://routing/{add,onadd,off}`), **v2RayTun**
   (`routing` = base64 Xray JSON). Value **high** for providers (compliance/split-tunnel). Complexity
   **M**. Place **core** (parse + apply in `config`) + **app** (show as a locked profile).
3. **App-management subscription headers** — provider controls client behaviour. Apps: **Happ**
   (`routing-enable`, `custom-tunnel-config`, `socks-auth-*`, `http-auth-*`, `tun-type`,
   `new-url`, `new-domain`). Value **high** (providers), **M** complexity. Place **core**
   (subscription meta → `Settings`) + **app** (respect overrides).
4. **HWID limit response handling + provider anti-abuse flows** — react to
   `x-hwid-limit` / `x-hwid-active` / `x-hwid-max-devices-reached` / `x-hwid-not-supported`
   (Remnawave, v2RayTun/Remnawave convention) and support Happ's `providerid` / `installid` checks.
   Flamix already *sends* `x-hwid`/`x-device-os`/… but ignores the response. Value **high** (many
   panels). Complexity **M**. Place **core** (fetch/parse) + **app** (error/announce UI).
5. **Backup / restore (state + profiles)** — export/import all state (and optionally WebDAV). Apps:
   v2rayN (zip + WebDAV), v2rayNG, Hiddify. Value **high** (migration). Complexity **M**. Place
   **core** (serialize) + **app** (UI) + **packaging** (optional).
6. **Auto-update of the app and of cores/geo files** — in-app update of the binary, external cores
   and geo/rule data. Apps: Happ, v2rayN, v2rayNG, Hiddify. Value **high**. Complexity **M**. Place
   **app** + **packaging** (release manifest) + **core** (download/verify).

## Medium value

7. **Advanced announcements** — richer provider messages: colors, buttons/links, expiry notices.
   Apps: **Happ** (`sub-info-color/text/button-text/button-link`, `sub-expire*`), **v2RayTun**
   (`announce` RGB colors + `announce-url`). Value **medium**. Complexity **S/M**. Place **core**
   (parse) + **app** (render).
8. **Encrypted subscription *content*** (AES-128-GCM body + `encrypt-tag` header, key id via `?key=`)
   — Happ. Value **medium** (depends on provider adoption). Complexity **M**. Place **core**
   (`subscription/fetch`). Requires an AES-GCM dependency.
9. **Encrypted links** — `happ://crypt4/<…>`, `happ://crypt5/<…>`, `v2raytun://crypt/<…>` (RSA-4096).
   Value **medium**. Complexity **M**. Place **core** (`deeplink`). Needs RSA + embedded keys; may be
   deferred.
10. **Custom per-subscription request headers** — user-defined header JSON sent on subscription
    requests (v2rayN). Value **medium**. Complexity **S**. Place **core** (`Subscription.user_agent`
    exists; add a header map) + **app** (editor).
11. **Multiple URLs merged into one group + per-subscription URL filtering** — v2rayN `MoreUrl`,
    Hiddify remote-line expansion; plus regex/name filter and v2RayTun `network-filter`. Value
    **medium**. Complexity **S**. Place **core** (`subscription`) + **app**.
12. **Bandwidth speed test** — throughput test in addition to latency. Apps: v2rayN (batch page size,
    inter-batch delay), v2rayNG. Value **medium**. Complexity **M**. Place **core** (`ping`) + **app**.
13. **DNS hosts map** (per-profile) — static domain→IP overrides. Apps: Happ (`DnsHosts`), v2rayN.
    Value **medium**. Complexity **S**. Place **core** (`config`) + **app**.
14. **Certificate pinning** — pin server cert by SHA-256 fingerprint or full PEM (v2rayN, Xray).
    Value **medium** (security-sensitive users). Complexity **M**. Place **core** (`config`/model).
15. **Name/regex + network filter for servers within a subscription** (v2rayN `Filter`, v2RayTun
    `network-filter`). Value **medium**. Complexity **S**. Place **core** (`subscription`) + **app**.
16. **QR code import and link export** — scan/generate (Happ, v2rayN, v2rayNG, Hiddify). Value
    **medium**. Complexity **S/M**. Place **app** (+ a QR dependency).
17. **Subscription converter support** — route a URL through a converter service with a target format
    (v2rayN `ConvertTarget` + converter URL). Value **medium** (legacy panels). Complexity **S**.
    Place **core** (`subscription`) + **app**.

## Low value

18. **ICMP ping** — true ICMP latency (Happ). Value **low/medium**. Complexity **M** (raw sockets /
    elevated, disable tunnel). Place **sys**.
19. **UDP connectivity test** (ntp/dns/stun/mcbe) — v2rayN. Value **low**. Complexity **M**. Place
    **core** (`ping`).
20. **Ping on launch** — auto-test all servers at startup (Happ). Value **low/medium**. Complexity
    **S**. Place **app**.
21. **Proxy chain (front/landing proxy)** — chain two nodes (v2rayN, Hiddify extra-security).
    Value **medium** but niche. Complexity **L**. Place **core** (`config`).
22. **Anytls protocol** — v2rayN. Value **low**. Complexity **S**. Place **core** (`link`, `config`).
23. **Trusted-link anti-impersonation check** for provider links (Hiddify). Value **low**. Complexity
    **S**. Place **app**.
24. **Per-app proxy** — Android-only (v2rayNG, Hiddify); not applicable to the desktop phases.
    Value **low** now. Complexity **L**. Place **packaging/mobile**.
25. **Desktop/Android widgets** — Happ/v2rayNG. Value **low**. Complexity **M**. Place **app**.

## Compatibility we should accept but don't yet

### Subscription headers / metadata to accept

- `fallback-url` (**Happ**) — add as an alias of our `profile-backup-urls`.
- `routing` + `happ://routing/{add,onadd,off}` and v2RayTun's base64 `routing` — see gap #2.
- App-management keys (**Happ**): `routing-enable`, `custom-tunnel-config`, `socks-auth-mode/user/password`,
  `http-auth-mode/user/password`, `tun-type`, `new-url`, `new-domain`. See gap #3.
- Announcement keys: `announce-url` (**v2RayTun**), `update-always` (**v2RayTun**),
  `network-filter` (**v2RayTun**), `sub-info-color/text/button-text/button-link`,
  `sub-expire`, `sub-expire-button-link` (**Happ**). See gaps #7/#11.
- `providerid` (**Happ**) and `installid` in the subscription URL/fragment (**Happ limited links**).
- `subscription-refill-date` (**Remnawave**) — traffic reset date; parse and ignore gracefully.
- Response headers to interpret: `encrypt-tag` (**Happ**), `x-hwid-limit`,
  `x-hwid-active`, `x-hwid-max-devices-reached`, `x-hwid-not-supported` (**Remnawave/v2RayTun**).
- `serverDescription` of a link/JSON `meta` (**Happ**) — a per-server subtitle; we could map it to a
  new optional `Server` field.

### Deep links / schemes to accept

- `happ://add/<url>` (widely used, **undocumented** on happ.su) and `happ://crypt4/<…>`,
  `happ://crypt5/<…>` (**Happ**).
- `happ://routing/{add,onadd,off}/<base64>` (**Happ**).
- `v2raytun://import/<…>` (already planned via T02) and `v2raytun://crypt/<…>` (**v2RayTun**).
- `v2rayng://install-sub?url=…` and `v2rayng://install-config?url=…`, with the **URL fragment as the
  profile name** (**v2rayNG**).
- `hiddify://install-config?url=…&name=…#<name>` (and the path form `hiddify://<url>#<name>`),
  plus `v2ray://`, `v2rayn://`, `v2rayng://`, `clash://`, `clashmeta://`, `sing-box://`
  (all with `?url=`&`name=`/fragment) (**Hiddify**); in particular
  `sing-box://import-remote-profile?url=…` and `clash://install-config?url=…`.

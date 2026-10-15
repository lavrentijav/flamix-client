# Feature matrix

Legend: ✅ confirmed from an official source; ❌ not present / not documented; **?** not confirmed
(no official source found). Flamix = planned per `docs/subscription-headers.md`,
`crates/flamix-core/src/model.rs` and `docs/tasks/*.md` (❌ = not in the current plan, not necessarily
impossible).

Sources for each client are in `happ.md`, `v2raytun.md`, `v2rayn.md`, `v2rayng.md`, `hiddify.md`.

| Feature | Happ | v2RayTun | v2rayN | v2rayNG | Hiddify | Flamix |
|---|---|---|---|---|---|---|
| **Platforms** | | | | | | |
| Windows desktop | ✅ | ✅ | ✅ | ❌ | ✅ | ✅ |
| macOS desktop | ✅ | ? | ✅ | ❌ | ✅ | ✅ |
| Linux desktop | ✅ | ? | ✅ | ❌ | ✅ | ✅ |
| Android | ✅ | ✅ | ❌ | ✅ | ✅ | ❌ (planned) |
| iOS | ✅ | ✅ | ❌ | ❌ | ✅ | ❌ (planned) |
| TV (Android/Apple TV) | ✅ | ? | ❌ | ? | ❌ | ❌ |
| **Cores / protocols** | | | | | | |
| Xray core | ✅ | ? | ✅ | ✅ | ✅ (opt-in) | ✅ |
| sing-box core | ✅ | ? | ✅ | ❌ | ✅ | ✅ |
| VLESS (+ Reality) | ✅ | ? | ✅ | ✅ | ✅ | ✅ |
| VMess | ✅ | ? | ✅ | ✅ | ✅ | ✅ |
| Trojan | ✅ | ? | ✅ | ✅ | ✅ | ✅ |
| Shadowsocks | ✅ | ? | ✅ | ✅ | ✅ | ✅ |
| Hysteria2 / hy2 | ✅ | ? | ✅ | ✅ | ✅ | ✅ |
| TUIC | ❌ | ? | ✅ | ✅ | ✅ | ✅ |
| WireGuard | ? | ? | ✅ | ✅ | ✅ | ✅ |
| SOCKS / HTTP proxy | ✅ | ? | ✅ | ✅ | ✅ | ✅ |
| Anytls | ❌ | ? | ✅ | ? | ❌ | ❌ |
| XHTTP transport | ? | ? | ? | ? | ? | ✅ |
| **Modes** | | | | | | |
| System proxy | ? | ? | ✅ | ❌ | ? | ✅ |
| TUN | ✅ | ? | ✅ | ✅ | ✅ | ✅ |
| Per-app proxy | ? | ? | ❌ | ✅ | ✅ | ❌ |
| Local SOCKS+HTTP inbound | ✅ | ? | ✅ | ✅ | ✅ | ✅ |
| LAN sharing | ✅ | ? | ✅ | ? | ✅ | ✅ |
| **Subscriptions** | | | | | | |
| Subscription URL (base64/plain) | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| JSON (Xray/sing-box) | ✅ | ? | ✅ | ✅ | ✅ | ✅ |
| Clash / Mihomo YAML | ❌ | ? | ✅ | ? | ✅ | ✅ |
| SIP008 (Shadowsocks) | ❌ | ? | ✅ | ? | ? | ❌ |
| `subscription-userinfo` traffic/expiry | ✅ | ✅ | ❌ | ❌ | ✅ | ✅ |
| Per-server quota | ❌ | ❌ | ❌ | ❌ | ❌ | ✅ (ext) |
| `profile-title` | ✅ | ✅ | ❌ | ❌ | ✅ | ✅ |
| `profile-update-interval` | ✅ | ✅ | ❌ (own setting) | ❌ | ✅ | ✅ |
| `support-url` / `profile-web-page-url` | ✅ | ✅ (site only) | ❌ | ❌ | ✅ | ✅ |
| `announce` | ✅ | ✅ (+ color/url) | ❌ | ❌ | ❌ | ✅ |
| `routing` header | ✅ | ✅ | ❌ | ❌ | ❌ | ❌ |
| App-management headers | ✅ | ❌ | ❌ | ❌ | ❌ | ❌ |
| HWID request headers (`x-hwid`, `x-device-os`, `x-ver-os`, `x-device-model`) | ✅ (own flow) | ✅ | ✅ (custom hdr) | ❌ | ❌ | ✅ |
| Custom request headers per subscription | ❌ | ❌ | ✅ | ❌ | ❌ | ❌ (UA only) |
| Backup / fallback subscription URLs | ✅ (`fallback-url`) | ❌ | ❌ (multi-URL) | ❌ | ❌ | ✅ (ext) |
| Multiple URLs merged in one group | ❌ | ❌ | ✅ (`MoreUrl`) | ? | ✅ (remote lines) | ❌ |
| Subscription converter | ❌ | ❌ | ✅ | ? | ? | ❌ |
| Name / regex filter | ❌ | ✅ (`network-filter`) | ✅ | ? | ? | ❌ |
| Auto-update interval | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| Encrypted subscription link | ✅ (crypt4/5) | ✅ (crypt) | ❌ | ❌ | ❌ | ❌ |
| Encrypted subscription content (AES-GCM) | ✅ | ? | ❌ | ❌ | ❌ | ❌ |
| **Deep links** | | | | | | |
| Own import scheme | ✅ (`happ://crypt*`, `happ://routing/*`) | ✅ (`v2raytun://import/*`) | ✅ (`v2rayn://`, clipboard) | ✅ (`v2rayng://`) | ✅ (`hiddify://`) | ✅ (`flamix://`) |
| Accepts `happ://add/…` | ✅ | ? | ❌ | ❌ | ? | ✅ |
| Accepts `v2raytun://import/…` | ❌ | ✅ | ❌ | ❌ | ✅ | ✅ |
| Accepts `v2rayng://install-sub?url=` | ❌ | ? | ❌ | ✅ | ✅ | ✅ |
| Accepts `clash://` / `sing-box://` | ❌ | ? | ✅ (clipboard) | ? | ✅ | ✅ |
| Routing profile deep link | ✅ | ? | ❌ | ❌ | ❌ | ❌ |
| **Routing / DNS** | | | | | | |
| Built-in routing presets | ✅ (profiles) | ✅ (header) | ✅ | ✅ | ✅ | ✅ |
| Custom domain/IP rules | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| Process rules (TUN) | ❌ | ? | ✅ | ? | ✅ | ✅ |
| Geo files (geoip/geosite) | ✅ | ? | ✅ | ✅ | ✅ | ✅ |
| Remote rule-sets / `.srs` | ? | ? | ✅ | ? | ✅ | ✅ |
| Remote + direct DNS | ✅ | ? | ✅ | ? | ✅ | ✅ |
| Fake-IP | ✅ | ? | ✅ | ? | ? | ✅ |
| DNS hosts map | ✅ | ? | ✅ | ? | ? | ❌ |
| Block QUIC | ? | ? | ? | ? | ? | ✅ |
| Domain strategy | ✅ | ✅ | ✅ | ? | ? | ✅ |
| **Ping / tests** | | | | | | |
| TCP ping | ✅ | ? | ✅ | ✅ | ✅ | ✅ |
| URL / via-proxy ping (GET/HEAD) | ✅ | ? | ✅ | ✅ | ✅ | ✅ |
| ICMP ping | ✅ | ? | ❌ | ? | ? | ❌ |
| UDP test (dns/stun/ntp/mcbe) | ❌ | ? | ✅ | ? | ? | ❌ |
| Bandwidth speed test | ? | ? | ✅ | ? | ? | ❌ |
| Ping all | ✅ | ? | ✅ | ✅ | ✅ | ✅ |
| Ping on launch | ✅ | ? | ❌ | ? | ? | ❌ |
| **UI / UX** | | | | | | |
| Grouping by subscription | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| Sort by ping / name | ? | ? | ✅ | ✅ | ✅ (delay) | ✅ |
| Search / filter server list | ? | ? | ✅ | ✅ | ? | ✅ |
| Per-server traffic display | ❌ | ❌ | ❌ | ❌ | ❌ | ✅ (ext) |
| Traffic statistics page | ? | ? | ✅ | ? | ✅ | ✅ |
| Tray icon | ? | ? | ✅ | ❌ | ✅ | ✅ |
| Autostart | ? | ? | ✅ | ? | ✅ | ✅ |
| Desktop widgets | ❌ | ? | ❌ | ✅ (Android) | ? | ❌ |
| Light / dark theme | ? | ? | ✅ | ✅ | ✅ | ✅ |
| RU + EN UI | ✅ (site) | ✅ (docs) | ✅ | ✅ | ✅ (many) | ✅ |
| QR scan import | ✅ | ? | ✅ | ✅ | ✅ | ❌ |
| QR / link export | ✅ | ? | ✅ | ✅ | ✅ | ❌ |
| Backup / restore | ? | ? | ✅ (zip/WebDAV) | ✅ | ✅ | ❌ |
| Auto-update of the app | ✅ | ? | ✅ | ✅ | ✅ | ❌ |
| Auto-update of cores / geo | ✅ | ? | ✅ | ✅ | ✅ | ❌ |
| Live session up/down speed | ? | ? | ✅ | ✅ | ✅ | ✅ |
| Policy groups / auto-select | ❌ | ❌ | ✅ | ? | ✅ | ❌ |
| Proxy chain (front/land) | ❌ | ❌ | ✅ | ? | ✅ (extra-security) | ❌ |
| **Security** | | | | | | |
| uTLS fingerprint | ✅ | ? | ✅ | ✅ | ✅ | ✅ |
| Certificate pinning | ? | ? | ✅ | ? | ? | ❌ |
| Anti-impersonation trusted-link check | ❌ | ❌ (branding) | ❌ | ❌ | ✅ | ❌ |

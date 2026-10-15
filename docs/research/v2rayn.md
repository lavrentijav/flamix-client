# v2rayN — feature research

v2rayN is the open-source desktop client (C#/.NET, Avalonia) maintained by 2dust. Sources: the
repository <https://github.com/2dust/v2rayN> (README, wiki, source at the commit cloned during this
research) and its wiki <https://github.com/2dust/v2rayN/wiki>.

## Platforms

| OS | x64 | x86 | arm64 | riscv64 | loong64 |
|---|---|---|---|---|---|
| Windows | ✅ | ✅ | ✅ | – | – |
| Linux | ✅ | – | ✅ | ✅ | ✅ |
| macOS | ✅ | – | ✅ | – | – |

Source: <https://github.com/2dust/v2rayN/blob/master/README.md>

## Protocols & transports

Subscription/wiki lists the supported protocols: **VMess, Shadowsocks, SOCKS, VLESS, Trojan, Hysteria2
(Hy2), TUIC, WireGuard, Anytls**.
Source: <https://github.com/2dust/v2rayN/wiki/Description-of-subscription>

Multiple cores are supported: **Xray**, **v2fly**, **mihomo (Clash.Meta)**, **sing-box**, plus
external cores **hysteria2, naiveproxy, tuic, juicity, brook, overtls, shadowquic**. VLESS link uses
the [XTLS share-link standard](https://github.com/XTLS/Xray-core/discussions/716).
Sources: <https://github.com/2dust/v2rayN/wiki/List-of-supported-cores>,
<https://github.com/2dust/v2rayN/wiki/Description-of-VLESS-share-link>,
<https://github.com/2dust/v2rayN/wiki/Description-of-VMess-share-link>

Transport knobs exposed in settings: mKCP (`KcpItem`), gRPC (`GrpcItem`), Xray mux (`Mux4RayItem`),
sing-box mux (`Mux4SboxItem`), fragmentation (`Fragment4RayItem`), Hysteria hop interval.
Source: <https://github.com/2dust/v2rayN/wiki/Description-of-some-parameters>

## Modes

- **System proxy** with four policies: clear proxy, auto-configure proxy, do-not-change, and **PAC**
  mode (Windows). Source: <https://github.com/2dust/v2rayN/wiki/Description-of-system-proxy-routing>
- **TUN mode** (`TunModeItem`): `EnableTun`, `AutoRoute`, `StrictRoute`, `Stack`
  (`system`/`gvisor`/`mixed`, default gvisor; Xray supports gvisor only), `Mtu` (default 9000),
  `EnableIPv6Address`, `IcmpRouting` (`rule`/`direct`/`unreachable`/`drop`/`reply`, sing-box only),
  `RouteExcludeAddress` (CIDRs excluded from auto-route), IPv4/IPv6 addresses. Admin rights are
  recommended at startup so TUN can start.
  Sources: <https://github.com/2dust/v2rayN/blob/master/v2rayN/ServiceLib/Models/Configs/ConfigItems.cs>,
  <https://github.com/2dust/v2rayN/wiki/Description-of-some-ui>
- **Custom config launcher**: for a custom (Clash/sing-box/etc.) config the user supplies the local
  Socks port; v2rayN then runs a front service that does routing/speed display before forwarding to
  that Socks port. Source: <https://github.com/2dust/v2rayN/wiki/Description-of-some-ui>
- **Local inbound** `InItem`: mixed SOCKS+HTTP port; second port, LAN port, PAC port, Xray API port,
  mihomo API port are derived (base, +1, +2, +3, +4, +5). Options: `UdpEnabled`, sniffing +
  `DestOverride` (`http`,`tls`), `RouteOnly`, `AllowLANConn`, user/pass auth.
  Sources: ConfigItems.cs, Description-of-some-ui.
- Per-app proxy is **not** a desktop feature; process-based routing is available in TUN via routing
  rules (`process`).

## Subscription formats & headers

### Accepted body formats

From the clipboard-import parser (also used for subscription content):

1. `https://` → treat as a new subscription;
2. protocol share links (standard schemes);
3. **SIP008** (Shadowsocks SIP008 JSON);
4. WireGuard config;
5. v2rayN internal share link (`v2rayn://…`);
6. multiple sing-box or v2ray custom configs;
7. a single sing-box or v2ray custom config;
8. Clash or Hysteria custom config.

Source: <https://github.com/2dust/v2rayN/wiki/Description-of-some-ui>

Subscription bodies may be plain or base64; the downloader tries both. The internal format is
`v2rayn://${ConfigType}/${url-safe base64 of ProfileItem JSON}` (also `v2rayn://policygroup/…` for
policy groups). Sources: <https://github.com/2dust/v2rayN/wiki/Description-of-subscription>,
<https://github.com/2dust/v2rayN/wiki/Description-of-some-ui>

### Response headers

v2rayN does **not** parse `subscription-userinfo` / `profile-title` / `profile-update-interval` /
`profile-web-page-url` / `announce` from the HTTP response. Searches across the source find no
handling of these response headers — only the response **body** is used.
(Verified by searching the cloned source; `SubscriptionHandler`/`DownloadService` only consume the
body.)

### Request headers / per-subscription options

Each subscription (`SubItem`) supports:

- **Custom request headers** as a JSON object, e.g. `{"X-hwid": "my_device"}` — values override
  defaults for all URLs in the group (used for HWID/anti-abuse panels).
- **User-Agent** (default `v2rayN/<version>`; many panels return different formats by UA).
- **MoreUrl** — additional subscription URLs used when a single group is desired (comma-separated;
  downloaded and merged; disables the subscription converter).
- **Filter** — regex alias filter applied after parsing.
- **AutoUpdateInterval** — auto-update period.
- **ConvertTarget** — optional subscription-converter target type.
- **PrevProfile / NextProfile / PreSocksPort** — front/landing proxy chain and pre-Socks port.
- **CustomCoreType** — per-subscription core override.

Sources: <https://github.com/2dust/v2rayN/blob/master/v2rayN/ServiceLib/Models/Entities/SubItem.cs>,
<https://github.com/2dust/v2rayN/blob/master/v2rayN/ServiceLib/Helper/HttpRequestHeadersHelper.cs>,
<https://github.com/2dust/v2rayN/blob/master/v2rayN/ServiceLib.Tests/Services/DownloadServiceHeadersTests.cs>,
<https://github.com/2dust/v2rayN/wiki/Description-of-some-ui>

## Deep links / URL schemes

- No OS-level URL-scheme registration was found in the repository at this checkout. Import happens
  via **clipboard** (Ctrl+V on non-input areas) and **QR code**.
- The internal share format `v2rayn://vless/<b64>` (and `v2rayn://http/…`, `v2rayn://policygroup/…`)
  is recognized by the clipboard importer.
Sources: <https://github.com/2dust/v2rayN/wiki/Description-of-some-ui>,
<https://github.com/2dust/v2rayN/wiki/Description-of-subscription>

## Routing

- Routing rule sets are JSON arrays of rules:
  `{"domain":[...], "ip":[...], "port":"...", "network":"tcp|udp", "outboundTag":"proxy|direct|block",
  "enabled":true, "remarks":"..."}`; proxy process rules use `process`.
  Source: <https://github.com/2dust/v2rayN/wiki/Description-of-custom-routing-rules>
- One-click import of the default rule set; import rules from file / clipboard / subscription URL;
  custom sing-box rule-set sources (local or remote JSON).
- Domain strategy (`DomainStrategy` / `DomainStrategy4Singbox`) is coupled to the rule set.
- Common usage: system proxy = auto, routing = "bypass mainland".
Sources: <https://github.com/2dust/v2rayN/wiki/Description-of-some-ui>,
<https://github.com/2dust/v2rayN/wiki/Description-of-system-proxy-routing>

## DNS

Rich DNS configuration (`SimpleDNSItem` + custom DNS):

- Direct DNS, Remote DNS, Bootstrap DNS (IP form), direct/proxy target resolution strategies;
- `FakeIP` / `GlobalFakeIp` + FakeIP range (sing-box TUN only);
- block SVCB/HTTPS queries (sing-box TUN only);
- parallel query and serve-stale (Xray only);
- custom DNS hosts (`example.com 1.2.3.4 5.6.7.8`, `keyword:…`), system-hosts injection;
- sing-box routing phase resolve vs outbound resolve differences.
Source: <https://github.com/2dust/v2rayN/wiki/Description-of-some-ui>,
<https://github.com/2dust/v2rayN/blob/master/v2rayN/ServiceLib/Models/Configs/ConfigItems.cs>

## Ping / speed test

- TCP ping and **real ping** (through the proxy to a test URL). `SpeedTestItem`: timeout, test URL,
  speed-ping URL, mixed concurrency, IP API URL, UDP test target, **batch page size** (default 1000)
  and inter-batch delay (default 1 s).
- **UDP test** addresses with protocols `ntp`, `dns`, `stun`, `mcbe`.
- Server list has latency and speed columns (sortable); `EnableStatistics` + real-time speed display.
Sources: <https://github.com/2dust/v2rayN/blob/master/v2rayN/ServiceLib/Models/Configs/ConfigItems.cs>,
<https://github.com/2dust/v2rayN/wiki/Description-of-some-ui>

## UI features

- Server list: sortable columns (click to sort asc/desc; invalid latency/speed always last),
  draggable/reorderable columns with saved layout, fuzzy name filter, regex info-log filter.
- Tray icon with a right-click server switch menu (limited to `TrayMenuServersLimit`, default 20);
  close-button behaviour configurable (hide/minimize/quit), auto-hide on startup, macOS dock toggle.
- Drag-drop sort, double-click to activate, global hotkeys, themes/colors/font/size, UI language.
- QR code scan/generate, share links, clipboard paste, server/subscription groups & grouping.
- **Policy groups**: Xray supports `most stable`, `lowest latency`, `load balance`, `random`;
  sing-box supports `lowest latency`; failover = high-tolerance lowest-latency (5000 ms).
- **Proxy chains**: front proxy → exit proxy.
- **Backup/restore**: whole `guiConfigs` folder and **WebDAV** sync.
- **Auto-update**: subscriptions (per interval), Geo files (interval, 0 = off), cores and the app
  itself (downloads from GitHub, forced through the proxy); check-pre-release options.
Sources: ConfigItems.cs,
<https://github.com/2dust/v2rayN/wiki/Description-of-some-ui>,
<https://github.com/2dust/v2rayN/wiki/Description-of-some-parameters>

## Security features

- **Root certificate provider** (`system` | `mozilla` | `chrome`) for update/cert-fetch validation
  against MITM; optional **certificate pinning** by SHA-256 fingerprint (Xray only) or a full PEM
  chain, plus a helper to fetch the server certificate/chain.
- Custom headers and root-cert handling added partly to mitigate update/download MITM.
Sources: <https://github.com/2dust/v2rayN/wiki/Description-of-some-ui>

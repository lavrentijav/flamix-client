# Happ — feature research

Happ ("Happ - Proxy Utility", by Flyfrog LLC) is a closed-source proxy client built on **Xray**. Its
official site is a GitBook at <https://www.happ.su/main> (the project also advertises a new domain
`https://happ.info`, per its home page). All claims below are sourced; anything not found in official
docs is marked **unverified**.

## Platforms

- Desktop: Windows 10 (1809+) / 11, macOS 13+ (`.dmg`; a separate App Store "Catalyst" build needs
  macOS 15+), Linux (Ubuntu 22.04+, Debian 11.6+, RHEL 8.10+, glibc 2.31+). Desktop is built with
  **Qt 6**; mobile (iOS/Android) is native (Swift/Kotlin).
  Source: <https://www.happ.su/main/faq/requirements.md>
- Mobile: iOS/iPadOS 15+, Android 5.0+.
- TV: Android TV 5.0+ and Apple TV (tvOS 15+).
- Legacy build 1.0.2 exists for old OSes (Win7/8 etc.).
  Source: <https://www.happ.su/main/faq/requirements.md>
- Download links (App Store global/RU, TestFlight, Google Play `com.happproxy`, GitHub
  `Happ-proxy/happ-desktop`, `Happ-proxy/happ-android`): <https://www.happ.su/main/readme.md>

## Protocols & transports

Documented core protocols: **VLESS (Reality)**, **VMess**, **Trojan**, **Shadowsocks**, **SOCKS5**,
**Hysteria2** (`hysteria2://` and `hy2://`). No TUIC/WireGuard in the "Adding Configuration" list
(WireGuard appears only in a *serverDescription* example link, so its support is ambiguous).
Sources: <https://www.happ.su/main/faq/adding-configuration-subscription.md>,
<https://www.happ.su/main/dev-docs/examples-of-links-and-parameters.md>

Transport/protocol parameters taken from links (per the examples):
`type` (tcp/ws/grpc/httpupgrade/xhttp…), `security` (`tls`/`reality`/`none`), `sni`, `fp` (uTLS
fingerprint, e.g. `chrome`), `alpn`, `flow` (`xtls-rprx-vision`, `xtls=2`), `pbk`/`sid`/`spx`
(Reality), `headerType`, `path`, `host`, `encryption`, `insecure`. Hysteria2 extras: `auth`
(userpass `user:pass`), multi-port `port=1234,5000-6000,7044`, `obfs=salamander`, `obfs-password`,
`mportHopInt` (port-hop interval), and `pinSHA256`. DPI tricks: `fragment=length,interval,packets[,maxSplit]`
and `noises=type,packet,delay[,applyTo]`.
Sources: <https://www.happ.su/main/dev-docs/examples-of-links-and-parameters.md>,
<https://www.happ.su/main/faq/hysteria2.md>, <https://www.happ.su/main/dev-docs/app-management.md>
(SOCKS5 parsing: plain, partial-base64, full-base64 — see examples page).
- JSON configs (raw Xray/sing-box config or array) are accepted and passed **1:1** to the core
  ("Direct Passthrough"); standard Happ routing/UI settings do not apply to such a JSON.
  Source: <https://www.happ.su/main/dev-docs/examples-of-links-and-parameters.md>

## Modes

- **Xray core** for everything (the app is "built on Xray"). For TUN it can use one of several
  tunnels; see below.
- **TUN** core selection via the `tun-type` setting: `singbox`, `tun2proxy`, `default` (Happ's own
  TUN) or `xray` (desktop only). Source: <https://www.happ.su/main/dev-docs/app-management.md>
- **Local SOCKS/HTTP inbound**: documented ports `10808` (SOCKS5) and `10809` (HTTP) when "Allow LAN
  Connections" is on; the same ports are the local proxy. Source:
  <https://www.happ.su/main/faq/local-network-connections.md>
- **Allow LAN Connections**: share the proxy over LAN; shows Current IP + SOCKS5/HTTP ports.
  Source: <https://www.happ.su/main/faq/local-network-connections.md>
- Per-app proxy: not documented on the site (**unverified**).

## Subscription formats & headers

Happ accepts: standard subscription URLs (plain / base64 list of share links), **JSON arrays / raw
configs** (Sing-box / V2Ray), and its **encrypted** forms (`happ://crypt4/`, `happ://crypt5/`, see
below). Sources: <https://www.happ.su/main/faq/adding-configuration-subscription.md>,
<https://www.happ.su/main/dev-docs/examples-of-links-and-parameters.md>

Metadata can be delivered **two ways** (headers win; body lines are prefixed `#key: value`, taken
from the leading comment block):
Source: <https://www.happ.su/main/dev-docs/app-management.md>

### Standard subscription headers / body keys

| Key | Value | Notes |
|---|---|---|
| `profile-update-interval` | integer **hours** (multiple of 1). Body: `#profile-update-interval: 1` | auto-update schedule |
| `profile-title` | text **or** `base64:<b64>`; max **25** chars | subscription name |
| `subscription-userinfo` | `upload=N; download=N; total=N; expire=UNIX` (`;`-separated) | quota; `total=0`/`expire=0` = unlimited/never |
| `support-url` | URL (Telegram icon if Telegram link) | support button |
| `profile-web-page-url` | URL | "open site" button |
| `announce` | text **or** `base64:<b64>`; max **200** chars | announcement above the list |
| `routing-enable` | `true`/`1` enable; any other non-empty value disables | global routing on/off |

Source: <https://www.happ.su/main/dev-docs/app-management.md>

### "App management" headers (provider-controlled settings)

Same delivery (header or `#key: value` body line). Enable with `true`/`1`; disable with e.g. `0`/`false`.

- `custom-tunnel-config`: full sing-box tunnel JSON (desktop only).
- Local inbound auth:
  - `socks-auth-mode` = `auto` \| `manual` \| `from-json` \| `disable`; `socks-auth-user`,
    `socks-auth-password` (manual mode).
  - `http-auth-mode` = `auto` \| `manual` \| `from-json` \| `disable`; `http-auth-user`,
    `http-auth-password`.
- `tun-type` = `singbox` \| `tun2proxy` \| `default` \| `xray` (desktop only).

Source: <https://www.happ.su/main/dev-docs/app-management.md>

### Advanced headers (require a `providerid`)

- `new-url`: full replacement subscription URL (`#new-url: https://mynew-domain.com/3J3jrb4jfc`).
- `new-domain`: replace only the host of the subscription URL.
- `fallback-url`: backup subscription URL. Failover triggers if the main URL fails, returns
  **300–599**, or does not answer within **9 s**.
- `serverDescription` (per-server): caption under the server name, max **30** chars, appended to the
  remark after `?`: `...#title?serverDescription=<base64_text>`; in JSON it is
  `"meta": { "serverDescription": "..." }`.

Source: <https://www.happ.su/main/dev-docs/app-management.md>

### Advanced announcements

| Key | Type | Values |
|---|---|---|
| `sub-info-color` | string | `red` \| `blue` (default) \| `green` |
| `sub-info-text` | string | max 200 chars; `0`/empty disables; without it no info block |
| `sub-info-button-text` | string | max 25 chars; button hidden if absent |
| `sub-info-button-link` | string | URL or deeplink, opened without validation |
| `sub-expire` | bool | `true`/`1` enables expiry notification (shown 3 days before/after expiry) |
| `sub-expire-button-link` | string | "Renew" button link (hidden if absent) |

Source: <https://www.happ.su/main/dev-docs/app-management.md>

### Provider ID / HWID / device limits

- `providerid` — unique provider ID; can be passed:
  1. in the URL fragment/query: `https://your-domain.com/sub/123#?providerid={ProviderID}`,
  2. in the body as a comment: `#providerid {ProviderID}`,
  3. in the response header `providerid: {ProviderID}`.
  Advanced app-management parameters require it. Once a day the app GETs
  `https://check.happ-proxy.com/provider?id={ProviderID}` sending providerid, the subscription
  domain hash, HWID, OS name and version.
  Source: <https://www.happ.su/main/dev-docs/provider-id.md>
- **HWID links**: subscription URLs carrying an HWID; the app compares the link's HWID with the
  device's local HWID and only adds the link when they match. Creation is only via happ-proxy.com.
  Source: <https://www.happ.su/main/dev-docs/hwid-links.md>
- **Limited links**: URL gets an `InstallID`, e.g. `https://url.com/sub#Your_TITLE?installid=Abcd1234`.
  On add, the client hashes the domain (SHA-256) and GETs
  `https://check.happ-proxy.com/install?id={domain_hash}&installid={InstallID}`; the server checks the
  device HWID against the device limit and returns ProviderID + status. On failure it retries after
  the VPN is up.
  Source: <https://www.happ.su/main/dev-docs/limited-links.md>

## Deep links / URL schemes

Documented schemes (exact syntax):

- `happ://crypt4/<payload>` and `happ://crypt5/<payload>` — encrypted links. `crypt4` = RSA-4096
  (deprecated), `crypt5` = current. Keys are embedded in the app; the user cannot view/edit/share an
  encrypted subscription. Encryption API: `POST https://crypto.happ.su/api-v2.php` with
  `{"url":"https://your-url.com"}` returns the encrypted link (this is a **POST**, listed here only
  as the documented provider flow; no POST was made during this research).
  Source: <https://www.happ.su/main/dev-docs/crypto-link.md>
- `happ://routing/add/{base64}` — add a routing profile (first profile activates after geo files
  download; same-name profiles are overwritten).
- `happ://routing/onadd/{base64}` — add and activate a routing profile.
- `happ://routing/off` — disable routing.
  Source: <https://www.happ.su/main/dev-docs/routing.md>
- The routing link may also arrive via clipboard, QR, HTTP header `routing: happ://routing/...`, or as
  a raw line in the subscription body.
  Source: <https://www.happ.su/main/dev-docs/routing.md>
- `happ://add/<url>` is used by third parties (incl. Flamix's own README) but is **not documented**
  on happ.su → **unverified**.
- Subscription encryption (body): providers may return an **AES-128-GCM** encrypted body with the
  auth tag in the `encrypt-tag` HTTP header (`base64`), keyed by a `?key=keyNN` parameter in the
  subscription URL. IV is the literal `kkkkkkkkkkkk` (12 bytes); AES-128 key = 16 bytes. Test key ID
  `key02`, test AES key `key02:+]%4ij#P"/`.
  Source: <https://www.happ.su/main/dev-docs/encrypting-subscription-content.md>

## Routing

- Pre-installed GeoIP/GeoSite files; updated with the core version.
  Source: <https://www.happ.su/main/dev-docs/routing.md>
- Per-subscription routing profiles ("Control Panel"), each with its own lifecycle. Changes only
  apply after a reconnect; deleting a subscription cascades to its profiles and cached geo files.
  Source: <https://www.happ.su/main/dev-docs/routing.md>
- JSON subscriptions are restricted: 0 or 1 routing profile, bundled only, not user-editable.
  Source: <https://www.happ.su/main/dev-docs/routing.md>
- Profile JSON keys (default profile): `Name`, `GlobalProxy`, `RemoteDNSType` (`DoH`/`DoU`/…),
  `RemoteDNSDomain`, `RemoteDNSIP`, `DomesticDNSType`, `DomesticDNSDomain`, `DomesticDNSIP`,
  `Geoipurl`, `Geositeurl`, `DnsHosts` (map), `DirectSites`, `DirectIp`, `ProxySites`, `ProxyIp`,
  `BlockSites`, `BlockIp`, `DomainStrategy` (`AsIs`/`IPIfNonMatch`), `FakeDNS`, `LastUpdated`.
  Tags use Xray syntax (`geosite:cn`, `geoip:cn`, `geoip:private`, CIDRs).
  Source: <https://www.happ.su/main/dev-docs/routing.md>
- Geo download manager: aborts after 3 min, shows an error on the main screen and a red mark on the
  profile; profile updates download new geo files in the background before atomically swapping.
  Source: <https://www.happ.su/main/dev-docs/routing.md>

## DNS

- Routing profile DNS fields: `RemoteDNSType`/`RemoteDNSDomain`/`RemoteDNSIP` (proxied lookups) and
  `DomesticDNSType`/`DomesticDNSDomain`/`DomesticDNSIP` (direct lookups); `DnsHosts` static map;
  `FakeDNS` toggle. `DomainStrategy` selects resolve behaviour.
  Source: <https://www.happ.su/main/dev-docs/routing.md>
- For JSON subscriptions, "Remote DNS" is renamed **Tunnel DNS** (Tunnel DNS Type / Tunnel IP /
  Tunnel Domain) and is applied locally, not injected into the provider JSON.
  Source: <https://www.happ.su/main/dev-docs/routing.md>

## Ping / speed test

- Three ping types: **ICMP** (needs the tunnel temporarily off), **TCP** (direct; may only reach a
  CDN edge), **via Proxy** (through the active tunnel; most representative, includes TLS handshake).
  Via-Proxy has **GET** (default) and **HEAD** modes.
  Sources: <https://www.happ.su/main/dev-docs/ping.md>, <https://www.happ.su/main/faq/ping.md>
- Display mode: **Time (ms)** or **Status icon** (green check / red exclamation).
- "Ping on Launch" tests every server at app start.
- Ping can be run per subscription, per server (swipe right → speedometer), or for the current
  connection ("Timeout" on failure).
  Source: <https://www.happ.su/main/faq/ping.md>
- No dedicated bandwidth/speed-test described in the docs (**unverified**).

## UI features

- Main screen: subscription ("Server List") cards with per-server rows; traffic/expiry bar from
  `subscription-userinfo`; support + site buttons; announcement; tap speedometer to ping a
  subscription; swipe a server right for actions (share/ping); "+" for manual/URL/QR/subscription
  add. Sources: app-management/ping/share pages above.
- Share a server (non-encrypted): **Copy to clipboard**, **QR code**, **JSON**.
  Source: <https://www.happ.su/main/faq/share-configuration.md>
- Flags: the **first** emoji of a server name is used as the icon (regional-indicator flag; both raw
  emoji and UTF-8 `%F0%9F...` encodings; a smiley before the flag suppresses the icon).
  Source: <https://www.happ.su/main/dev-docs/emoji.md>
- Localization: site is published in EN/RU/ZH; app languages not enumerated (**unverified**).
- Tray, autostart, themes, widgets, session speed: not documented on happ.su (**unverified**).
- Android TV "Web Import": app shows a 5-digit UID; provider POSTs
  `{"data":"<base64>"}` to `https://check.happ.su/sendtv/{uid}`.
  Source: <https://www.happ.su/main/dev-docs/android-tv-api.md>

## Security features

- Encrypted links `happ://crypt4/` (RSA-4096, deprecated) and `happ://crypt5/` (current); encrypted
  subscriptions cannot be viewed, edited or shared in-app.
  Source: <https://www.happ.su/main/dev-docs/crypto-link.md>
- Encrypted subscription **content** with AES-128-GCM + `encrypt-tag` header, keyed by `?key=`.
  Source: <https://www.happ.su/main/dev-docs/encrypting-subscription-content.md>
- HWID / device-limit / provider-ID anti-abuse tied to happ-proxy.com.
  Sources: hwid-links.md, limited-links.md, provider-id.md
- No analytics: "does not collect any data" per home page.
  Source: <https://www.happ.su/main/readme.md>

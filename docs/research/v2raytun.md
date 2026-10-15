# v2RayTun — feature research

v2RayTun is a (closed-source) multi-platform proxy client. Official documentation is a GitBook at
<https://docs.v2raytun.com>. The marketing site `https://v2raytun.com` did not resolve from the
research environment (connection failure), so most facts come from the docs subdomain and its
`llms.txt`/`site-index`. Anything not found there is marked **unverified**.

## Platforms

- Docs and premium pages mention **iOS, Android and Windows** (the crypto-link page says the feature
  will ship "in IOS 2.4.2, then Android and Windows"; Premium+ says Android support since May 1).
  macOS/Linux builds are **unverified** from the docs on hand.
  Sources: <https://docs.v2raytun.com/overview/about-premium+.md>,
  <https://docs.v2raytun.com/for-developers/crypto-link.md>
- Store pages (App Store / Google Play) were reachable only as links, not fetchable from this
  environment; see `screenshots.md`.

## Protocols & transports

Not enumerated in the docs. The routing header uses Xray-style JSON (`domainStrategy`, rules with
`outboundTag`, `geosite:`/`geoip:`), which suggests an Xray-family core, but this is **unverified**.
Source: <https://docs.v2raytun.com/overview/supported-headers.md>

Import works with share links / configuration strings (any scheme handled by the client) via deep
link. Source: <https://docs.v2raytun.com/deep-link.md>

## Modes

Not documented in the available pages. The device-limit page implies a normal client UI; TUN /
system-proxy / per-app are **unverified**.

## Subscription formats & headers

Per the docs: v2RayTun supports **HTTP headers and body headers**, and **base64-encoded or raw** body
headers. Source: <https://docs.v2raytun.com/overview/supported-headers.md>

| Header | Value / format | Meaning |
|---|---|---|
| `profile-title` | raw string **or** `base64:<b64>` | subscription name |
| `subscription-userinfo` | `upload=N; download=N; total=N; expire=UNIX`; all keys optional, `;`-separated | quota / expiry |
| `profile-update-interval` | integer **hours** (e.g. `1` = hourly) | auto-update interval |
| `routing` | **base64** of an Xray-style routing JSON | forces a shared routing profile for this subscription; takes precedence over app routing and Direct Service |
| `announce` | raw **or** `base64:<b64>`; supports inline RGB colors `#RRGGBB` before a word | announcement text |
| `announce-url` | URL | opened when the announcement is tapped |
| `update-always` | `"true"` | update the subscription every time the app opens (ignores `profile-update-interval`; only when the VPN is off) |
| `network-filter` | `"true"` | hide configs by name depending on network: names containing `LTE`/`Only mobile` show only on mobile, `WiFi`/`Only Wifi` only on Wi-Fi, others always |

Source: <https://docs.v2raytun.com/overview/supported-headers.md>

Routing JSON example (decoded) includes: `domainStrategy`, `id`, `balancers`, `domainMatcher`
(`hybrid`), `rules[]` (each with `domain`, `ip`, `outboundTag` e.g. `direct`, `__name__`),
`name`. Source: <https://docs.v2raytun.com/overview/supported-headers.md>

### Device limit (request headers, HWID)

Since v2.3.5 v2RayTun sends these request headers, "first established in Remnawave":

- `X-HWID`
- `X-Device-OS`
- `X-Ver-OS`
- `X-Device-Model`
- `X-App-Version`

Supported panel: Remnawave. Source: <https://docs.v2raytun.com/overview/device-limit.md>

## Deep links / URL schemes

- `v2raytun://import/{configuration}` — import a config.
- `v2raytun://import/{subscription_link}` — import a subscription.
  Source: <https://docs.v2raytun.com/deep-link.md>
- `v2raytun://crypt/{ENCRYPTED}` — import an encrypted subscription (feature announced, "API will be
  available soon").
  Source: <https://docs.v2raytun.com/for-developers/crypto-link.md>

## Routing

Routing is delivered as a base64 JSON via the `routing` header (see above). It overrides the app's
own routing and the Direct Service, must be passed exactly as produced by the app, and its use is
logged. Source: <https://docs.v2raytun.com/overview/supported-headers.md>

## DNS / Ping / Speed test

Not documented in the available pages (**unverified**).

## UI features

- **Premium+** (paid, provider-facing): provider **branding** of the key's dedicated card,
  a provider info page with support/site links, an **anti-ban** domain-replacement system
  (swap the connection domain on all issued keys with one click), and a support prompt when a key
  fails. Source: <https://docs.v2raytun.com/overview/about-premium+.md>
- Preview screenshot of the main screen: <https://docs.v2raytun.com/overview/supported-headers.md>
- Languages: docs are English + Russian; app languages **unverified**.

## Security features

- Encrypted links with **RSA-4096**: `v2raytun://crypt/{ENCRYPTED}`; the subscription address is
  hidden from the user. Public key published in the docs. The key/API "will be available soon".
  Source: <https://docs.v2raytun.com/for-developers/crypto-link.md>
- No explicit AES/encrypt-tag subscription-body encryption documented (**unverified**).

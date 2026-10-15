# v2rayNG — feature research (short)

v2rayNG is the Android client (Kotlin) by 2dust, using **Xray/v2fly**. Sources: repo
<https://github.com/2dust/v2rayNG> and wiki <https://github.com/2dust/v2rayNG/wiki>.

## Platforms

- Android, min API 24. Source: <https://github.com/2dust/v2rayNG/blob/master/README.md>

## Protocols & transports

Xray/v2fly protocols and transports (share links same family as v2rayN).
Source: <https://github.com/2dust/v2rayNG/blob/master/README.md>

## Modes

- **VPN (TUN) mode** and **Proxy-only mode** (local SOCKS/HTTP on `127.0.0.1:10808`).
- **Per-app proxy** (`PerAppProxyActivity`, app picker) and **root mode** (`RootProxyManager`).
Sources: <https://github.com/2dust/v2rayNG/wiki/Mode>,
<https://github.com/2dust/v2rayNG/blob/master/V2rayNG/app/src/main/java/com/v2ray/ang/ui/perappproxy/PerAppProxyActivity.kt>

## Subscription formats & headers

- Subscription by URL with **auto-update** (WorkManager periodic tasks; min interval clamp,
  `autoUpdate` per subscription). Source:
  <https://github.com/2dust/v2rayNG/blob/master/V2rayNG/app/src/main/java/com/v2ray/ang/handler/SubscriptionUpdater.kt>
- No parsing of `subscription-userinfo` / `profile-title` / `profile-update-interval` /
  `profile-web-page-url` / `announce` response headers was found in the source (search of the cloned
  repo) — only the body is consumed. Config formats: standard Xray JSON + share links.

## Deep links / URL schemes

Registered in `AndroidManifest.xml`: scheme `v2rayng`, hosts `install-config` and `install-sub`.
Handling (`UrlSchemeActivity`):

- `v2rayng://install-sub?url=<url>` (+ optional `#fragment` = name)
- `v2rayng://install-config?url=<url>` (+ optional `#fragment` = name)
- also receives Android `ACTION_SEND` `text/plain` shares.

The `url` query value is URL-decoded before import; a missing URL fragment is re-appended from the
deep-link fragment. Sources:
<https://github.com/2dust/v2rayNG/blob/master/V2rayNG/app/src/main/AndroidManifest.xml>,
<https://github.com/2dust/v2rayNG/blob/master/V2rayNG/app/src/main/java/com/v2ray/ang/ui/UrlSchemeActivity.kt>

## Routing / DNS / ping / UI

- Custom routing rules (`RoutingEditActivity`), geo files (`geoip.dat`/`geosite.dat`, user-updatable),
  bypass LAN. Sources: repo source.
- DNS handling in VPN mode; core latency testing (`CoreTestService`). Sources: repo source.
- QR scan/import, server groups, subscription groups. Source: repo + wiki.

## Security features

None documented beyond Xray's transports (**unverified**).

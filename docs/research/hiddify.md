# Hiddify — feature research (short)

Hiddify app is a multi-platform proxy client based on **sing-box**. Sources: repo
<https://github.com/hiddify/hiddify-app> and the project docs at <https://hiddify.com>.

## Platforms

- Android, iOS, Windows, macOS, Linux (README badges/stores).
  Source: <https://github.com/hiddify/hiddify-app/blob/main/README.md>
- Also ships a "use Xray core when possible" option.
  Source: <https://github.com/hiddify/hiddify-app/blob/main/lib/features/profile/data/profile_parser.dart>

## Protocols & transports

VLESS, VMess, Reality, TUIC, Hysteria(2), WireGuard, SSH (plus the sing-box transport set).
Source: <https://github.com/hiddify/hiddify-app/blob/main/README.md>

## Subscription formats & headers

Accepted profile formats: **sing-box, V2ray, Clash, Clash Meta**; auto subscription update; shows
remaining days and traffic usage. Source: README.

Response headers parsed (allow-list in `profile_parser.dart`):

- `profile-title` (raw or `base64:`) — subscription name;
- `content-disposition` — fallback name (`filename="..."` and RFC 5987 `filename*=UTF-8''...`);
- `subscription-userinfo` — `upload`/`download` required; missing or `0` `total`/`expire` = unlimited;
- `profile-update-interval` — positive integer hours;
- `profile-web-page-url` and `support-url` — shown as provider link chips (opened with a
  trusted-link check, see below);
- `extra-security` — enables a chain mode (`warp`/`psiphon`) → `chain-status`;
- `fragment` — enables TLS fragmentation (full triplet `size,sleep,method` or
  `tlshello,size,sleep`).

Body metadata headers are read from the **first 10 lines** that start with `#` or `//` after
base64-decoding the content; remote HTTP headers take precedence over body headers. Name fallback
order: user override → `profile-title` → `content-disposition` → URL fragment → URL file name.
Source: <https://github.com/hiddify/hiddify-app/blob/main/lib/features/profile/data/profile_parser.dart>

## Deep links / URL schemes

Registered/parsed schemes (`link_parsers.dart`): `hiddify`, `v2ray`, `v2rayn`, `v2rayng`, `clash`,
`clashmeta`, `sing-box`.

- `hiddify://install-config?url=<url>#<name>` and `hiddify://<url>` (path form, fragment = name);
- `<scheme>://…?url=<url>&name=<name>` for the `v2ray`/`v2rayn`/`v2rayng`/`clash`/`clashmeta`/`sing-box`
  schemes (name also accepted from the fragment).
Source: <https://github.com/hiddify/hiddify-app/blob/main/lib/utils/link_parsers.dart>

## Routing / DNS / modes

- TUN mode, per-app routing (`features/route_rules/overview/package_names_page.dart`), route rules
  and rule sets, delay-based automatic node selection, remote profiles, dark/light themes.
- DNS overrides: `direct-dns-address`, `remote-dns-address`, `connection-test-url` are among the
  per-profile override keys.
Sources: README, <https://github.com/hiddify/hiddify-app/tree/main/lib/features/route_rules>,
<https://github.com/hiddify/hiddify-app/blob/main/lib/features/profile/data/profile_parser.dart>

## Security features

- **Trusted-link allow-list**: subscription-provided `profile-web-page-url` / `support-url` links
  open with a light confirmation only when they exactly match a curated list (`hiddify.com`,
  `github.com/hiddify`, `t.me/hiddify`); everything else shows an anti-impersonation warning.
  Source: <https://github.com/hiddify/hiddify-app/blob/main/lib/core/model/trusted_links.dart>
- Extra security chain modes (Warp / Psiphon) and TLS fragmentation via `extra-security` / `fragment`.

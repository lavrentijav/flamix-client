# Subscription format and headers

Flamix accepts what Happ, v2RayTun, v2rayN and Hiddify accept, plus two extensions:
**backup subscription URLs** and **per-server traffic**.

## Body

Tried in order:
1. base64 (standard or URL-safe alphabet, padding optional, line breaks ignored) of a
   newline separated list of share links;
2. plain text list of share links (one per line, `\r\n` allowed);
3. JSON: an Xray/sing-box config or an array of them — best effort: outbounds converted to servers;
4. Clash / Mihomo YAML with a `proxies:` list — best effort.

Lines starting with `#` at the top of a plain/base64 body are metadata lines in the form
`#key: value` (the same keys as the headers below, Happ-compatible), e.g.

    #profile-title: base64:8J+HtvCfh7ogTXkgVlBO
    #subscription-userinfo: upload=0; download=1073741824; total=107374182400; expire=1798761600
    #profile-backup-urls: https://mirror1.example/sub/abc, https://mirror2.example/sub/abc
    vless://...

Headers win over body metadata when both are present.

## Standard headers

| Header | Value | Meaning |
|---|---|---|
| `subscription-userinfo` | `upload=N; download=N; total=N; expire=TS` | Quota of the whole subscription; any key may be missing, `total=0` = unlimited, `expire=0` = never |
| `profile-title` | text or `base64:<b64>` | Subscription name |
| `profile-update-interval` | hours (integer) | Auto-update period |
| `support-url` | URL | "Support" button |
| `profile-web-page-url` | URL | "Open site" button |
| `announce` | text or `base64:<b64>` | Message shown above the server list |
| `content-disposition` | `attachment; filename="name"` (RFC 6266, `filename*=UTF-8''...` too) | Fallback name |

## Extension 1: backup subscription URLs

    profile-backup-urls: https://a.example/sub/token, https://b.example/sub/token

- Value: comma (or newline) separated absolute `http(s)` URLs, or `base64:<b64>` of such a list.
- Aliases accepted: `backup-urls`, `x-backup-urls`, `fallback-urls`.
- The client merges them into `Subscription::backup_urls` (deduplicated, user-entered URLs first,
  max 10). On update the client tries: the primary URL → the last working URL → each backup URL,
  stopping at the first 2xx response that yields at least one server. The UI shows which URL
  answered.

## Extension 2: per-server traffic

A provider can report quota per server (e.g. separate limits per location).

**a) Header** — `server-userinfo`, value `base64:<b64 of a JSON object>`:

```json
{
  "🇩🇪 Germany #1": "upload=0; download=5368709120; total=53687091200; expire=1798761600",
  "de1.example.com:443": { "upload": 0, "download": 5368709120, "total": 53687091200 },
  "b831381d-6324-4d53-ad4f-8cda48b30811": "download=1; total=2"
}
```

Keys are matched against each server in this order: exact name (remark) → `address:port` →
credential id (uuid for vless/vmess/tuic, password for trojan/ss/hysteria2). Values are either a
`subscription-userinfo` string or an object with the same keys.

**b) Share link parameter** — `userinfo=<url-encoded subscription-userinfo string>` in the query of
any share link (for `vmess://` base64 JSON: the `userinfo` field). Other clients ignore unknown
parameters, so it is safe to add:

    vless://uuid@de1.example.com:443?type=ws&security=tls&userinfo=download%3D5368709120%3Btotal%3D53687091200#🇩🇪 Germany

Per-server data from the header wins over the link parameter. The UI shows a small progress bar
with "remaining / total" under such a server; the subscription bar shows the subscription quota.

## Request

- Default `User-Agent`: `Flamix/<version> v2rayN/7.0 Happ/3.0` (panels select the share-link
  format by UA); user-overridable per subscription.
- Headers sent: `x-hwid` (stable random device id), `x-device-os`, `x-ver-os`, `x-device-model`
  (Happ-compatible HWID limits on Remnawave), `Accept-Encoding: gzip`.
- Extra user headers (`SubscriptionOptions::custom_headers`) are sent too and override the
  defaults above, except `Host`.

## Compatibility (Happ / v2RayTun / Remnawave)

Every key below is accepted both as an HTTP response header and as a `#key: value` body line
(plain/base64 body); headers win. `#providerid X` (space, no colon) is also accepted. All of it is
stored in `Subscription::provider` (`ProviderExtras`).

| Key | Source | Meaning |
|---|---|---|
| `fallback-url` | Happ | Alias of `profile-backup-urls` (merged into `Subscription::backup_urls`) |
| `providerid` | Happ | Provider id; header, `#providerid X` body line, or subscription URL fragment `#?providerid=X` |
| `routing` | Happ / v2RayTun | Routing profile: a `happ://routing/...` link, base64 Xray routing JSON (`rules[]` + `outboundTag`), or base64 Happ profile JSON → `ProviderExtras::routing` |
| `routing-enable` | Happ | `true`/`1` enable, any other non-empty value disables provider routing |
| `announce-url` | v2RayTun | URL opened when the announcement is tapped |
| `sub-info-text` | Happ | Info banner text (empty/`0` disables) |
| `sub-info-color` | Happ | Banner colour: `red` / `blue` (default) / `green` or `#RRGGBB` |
| `sub-info-button-text` | Happ | Banner button label |
| `sub-info-button-link` | Happ | Banner button link |
| `sub-expire` | Happ | Expiry notice (raw value kept in `ProviderExtras::expire_notice`) |
| `sub-expire-button-link` | Happ | "Renew" button link |
| `update-always` | v2RayTun | Refresh on every app start while disconnected |
| `network-filter` | v2RayTun | Comma-separated transport/protocol tokens; only matching servers are kept |
| `subscription-refill-date` | Remnawave | Traffic reset date: unix seconds, RFC 3339 or `YYYY-MM-DD` |
| `x-hwid-limit` | Remnawave | Device limit (response header) |
| `x-hwid-active` | Remnawave | Currently active devices (response header) |
| `x-hwid-max-devices-reached` | Remnawave | Device limit reached flag (response header) |
| `x-hwid-not-supported` | Remnawave | HWID not supported flag (response header) |
| `custom-tunnel-config` | Happ | Full sing-box tunnel JSON (desktop) |
| `socks-auth-mode`, `socks-auth-user`, `socks-auth-password` | Happ | Local SOCKS inbound auth |
| `http-auth-mode`, `http-auth-user`, `http-auth-password` | Happ | Local HTTP inbound auth |
| `tun-type` | Happ | `singbox` / `tun2proxy` / `default` / `xray` |
| `new-url` | Happ | Replacement subscription URL |
| `new-domain` | Happ | Replacement subscription host |

The app-management keys in the last block are stored verbatim in `ProviderExtras::app_overrides`
(lower-case key → raw value); applying them to the core configs is a separate task.

A raw `happ://routing/...` line in the body is parsed as a routing profile and removed before the
share links are parsed.

### Deep links accepted for import

`flamix://import|add/`, `happ://add/`, `happ://routing/{add,onadd,off}/<base64>`,
`v2raytun://import/`, `v2rayng://install-sub?url=..` and `v2rayng://install-config?url=..`
(fragment = name), `hiddify://install-config?url=..&name=..#name` and the path form
`hiddify://<url>#<name>`, `v2ray://`, `v2rayn://`, `clash://`, `clashmeta://`,
`sing-box://import-remote-profile?url=..` (all with `?url=` / `name=` / fragment), plus bare
`https://` subscription URLs and share links.

Encrypted links (`happ://crypt4/`, `happ://crypt5/`, `v2raytun://crypt/`) use proprietary keys and
are **not** supported: they are ignored with a log message.

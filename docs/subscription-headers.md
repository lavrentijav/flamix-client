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

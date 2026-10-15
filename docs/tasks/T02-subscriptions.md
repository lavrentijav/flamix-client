# T02 — subscriptions (download, headers, backup URLs, per-server traffic) + deep links

Read `AGENTS.md` first (build rule: everything via `scripts/ru1`). Contract: `crates/flamix-core/src/model.rs`.
**Authoritative spec: `docs/subscription-headers.md`** — implement all of it.

## Setup
    git -C /srv/dev/projects/flamix-client worktree add /srv/dev/projects/flamix-client-wt/T02 -b oc/T02
    cd /srv/dev/projects/flamix-client-wt/T02

## Files you own
- `crates/flamix-core/src/subscription/mod.rs` (split into submodules `meta.rs`, `body.rs`, `fetch.rs` as you like)
- `crates/flamix-core/src/deeplink.rs`
You may add dev-dependencies to `crates/flamix-core/Cargo.toml` (e.g. a tiny HTTP test server is
NOT needed: use `tokio::net::TcpListener` and write raw HTTP responses in tests).

## Dependency on T01
Share link parsing (`crate::link::parse_links`) is written in parallel by T01 and is a `todo!()`
stub in your branch. Call it — do not implement link parsing yourself. In your tests that would
reach it, mark them `#[ignore = "needs T01 link parser"]`, and test everything else
(meta, body decoding, URL fallback order, userinfo matching) with functions that do not need it.
Structure the code so that the per-server-traffic matching is a pure function, e.g.
`fn apply_server_userinfo(servers: &mut [Server], map: &HashMap<String, TrafficInfo>)`,
testable with hand-built `Server` values.

## Implement
- `parse_userinfo`: keys in any order, spaces optional, `;` or `,` separated, unknown keys ignored, numbers may be floats like `1.5e9` (truncate) — return `None` if no known key parsed.
- `parse_meta`: headers case-insensitive; `base64:` prefix decoding for title/announce/backup/server-userinfo; RFC 6266 filename (+ `filename*=UTF-8''..`) as title fallback; `#key: value` body lines (only the leading block of `#` lines of the *decoded* body); headers win.
- `decode_body`: base64 (std + url-safe, any padding, whitespace/newlines removed before decoding; only accept if the decoded text contains `://`), plain text, JSON (Xray config `outbounds[]` with protocol vless/vmess/trojan/shadowsocks -> rebuild share links; sing-box `outbounds[]` type vless/vmess/trojan/shadowsocks/hysteria2/tuic -> share links; array of configs), Clash YAML `proxies:` (types vless, vmess, trojan, ss, hysteria2, tuic) -> share links. JSON/YAML conversion is best effort: skip what you can't map.
- `update`: reqwest client (rustls, gzip, optional proxy from `FetchOptions.proxy`, timeout default 20s), headers from the spec (`User-Agent` = `sub.user_agent` or `DEFAULT_USER_AGENT`; `x-hwid` if given; `x-device-os` = `std::env::consts::OS`; `x-ver-os`, `x-device-model` best effort/empty-safe). Order of URLs: primary, last_working_url, backups (dedupe). Success = 2xx and ≥1 parsed server. On total failure set `last_error` (short, human readable, listing each URL's error) and return `Err(Error::Http)`, keeping old servers. On success: new servers with ids from `Server::compute_id(&sub.id, ...)` (T01 does that inside `parse_links(text, &sub.id)`), preserve `last_ping_ms` and `local_traffic` for ids that existed before, apply per-server userinfo (header map wins over link `userinfo` param that T01 put into `Server::traffic`), set traffic/title (only if `sub.name` is empty or equals the previous auto title — keep user names)/support/web/announce/interval (only if user hasn't set one: treat 0 as unset), merge backup URLs (max 10), `last_update = now`, `last_error = None`, `last_working_url`.
- `deeplink::parse_import` per its doc comment (all listed schemes, percent-decoding, base64-encoded url payloads that Happ sometimes uses: if the payload after `add/` isn't `http(s)://` try base64-decoding it). Share links (any scheme T01 supports: vless, vmess, trojan, ss, hysteria2, hy2, tuic, wireguard, wg, socks, socks5) -> `Import::Links`; multi-line text where lines are links -> `Import::Links`; `http(s)://` URL -> `Import::Subscription`.

## Tests (required)
userinfo variants; meta from headers vs body lines; base64 url-safe without padding; Clash YAML sample;
sing-box JSON sample; per-server userinfo matching by name / host:port / uuid / password; fallback
order with a local TcpListener serving: primary -> 500, backup1 -> 200 empty body, backup2 -> 200 with
links (ignored test if it needs T01) and a pure test of the URL ordering function; deeplink for every scheme.

## Done
    scripts/ru1 cargo fmt --all --check
    scripts/ru1 cargo clippy -p flamix-core --all-targets -- -D warnings
    scripts/ru1 cargo test -p flamix-core
Commit on `oc/T02`. Report: branch, commit hash, test results, list of ignored tests.

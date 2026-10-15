# T01 — share link parser/exporter + country detection

Read `AGENTS.md` first (build rule: everything via `scripts/ru1`). Contract: `crates/flamix-core/src/model.rs`.

## Setup
    git -C /srv/dev/projects/flamix-client worktree add /srv/dev/projects/flamix-client-wt/T01 -b oc/T01
    cd /srv/dev/projects/flamix-client-wt/T01

## Files you own
- `crates/flamix-core/src/link/mod.rs` (may split into `link/vless.rs`, `link/vmess.rs`, ... submodules)
- `crates/flamix-core/src/country.rs`
Do not edit other files (exception: add `#[serde(default)]` fields to model.rs only if unavoidable — report it).

## Implement
`parse_link`, `parse_links`, `to_link` (signatures in the stub), compatible with v2rayN / v2rayNG /
Happ / v2RayTun / Hiddify links:

- `vless://uuid@host:port?type=ws&security=tls&sni=..&host=..&path=..&fp=..&alpn=h2,http/1.1&flow=xtls-rprx-vision&pbk=..&sid=..&spx=..&serviceName=..&mode=..&headerType=..&encryption=none&allowInsecure=1&extra=<urlenc json>#name`
  (`security=reality` -> `Security::Reality` with `pbk` public_key, `sid` short_id, `spx` spider_x; `type`: tcp/raw(=tcp)/ws/grpc/httpupgrade/xhttp/splithttp(=xhttp)/h2/http(=h2)/kcp/quic).
- `vmess://` base64 JSON (v2rayN format: `v, ps, add, port, id, aid, scy, net, type, host, path, tls, sni, alpn, fp`; port/aid may be strings or numbers) AND the URI form `vmess://uuid@host:port?...` (same params as vless).
- `trojan://password@host:port?security=tls&sni=..&type=..#name` (security defaults to tls).
- `ss://` SIP002: `ss://base64(method:password)@host:port/?plugin=..#name` and userinfo given as plain `method:password` (percent-encoded), and legacy `ss://base64(method:password@host:port)#name`. Plugin string `name;opts` -> `plugin` + `plugin_opts`.
- `hysteria2://` and `hy2://`: `password@host:port?sni=..&insecure=1&obfs=salamander&obfs-password=..&mport=20000-30000&alpn=..&pinSHA256=..#name` (TLS always -> `Security::Tls`). Port may be a range `host:443,20000-30000` -> port=first, `ports`=rest.
- `tuic://uuid:password@host:port?congestion_control=bbr&udp_relay_mode=native&alpn=h3&sni=..&allow_insecure=1#name`.
- `wireguard://` / `wg://`: `privatekey@host:port?publickey=..&presharedkey=..&address=10.0.0.2/32,fd00::2/128&reserved=1,2,3&mtu=1280#name` (keys may be URL-encoded base64).
- `socks://` / `socks5://` (`user:pass@` plain or base64 userinfo) and `http://` / `https://` proxy links (`https` -> TLS).
- Remark (`#name`) percent-decoded; empty -> `host:port`. IPv6 hosts `[::1]:443`.
- Unknown query parameter `userinfo` (see `docs/subscription-headers.md`, "Extension 2 b") -> `Server::traffic` via the same format as `subscription-userinfo` (write a private parser here; T02 has its own public one — duplication is fine).
- `Server::id` = `Server::compute_id(scope, address, port, &credentials)`; `country` = `country::detect_country(&name)`; `raw_link` = the input link trimmed.
- `parse_links`: also skip empty lines and lines starting with `#`; strip a UTF-8 BOM.
- `to_link`: canonical v2rayN-style link. `vmess` -> base64 JSON form. Round-trip test: `parse_link(to_link(parse_link(x)))` equals `parse_link(x)` except `raw_link`.
- Errors: `Error::Unsupported` for unknown schemes, `Error::Parse` for malformed links. Never panic on any input (add a test with garbage strings, truncated links, non-UTF-8-ish base64).

`country.rs`: `detect_country` — regional indicator pairs first (🇩🇪 -> "DE"; also 🇬🇧 etc.), then a
table of ~60 common countries' Russian and English names (case-insensitive, word match), then a
standalone 2-letter upper-case ISO code token (`DE-1`, `[NL]`, `NL 2`; ignore "VPN"-like non-codes by
checking against the ISO list). `flag_emoji("de")` -> "🇩🇪" (case-insensitive; invalid -> "").

## Tests (required)
At least 3 real-world samples per protocol (reality+vision, ws+tls, grpc, xhttp with extra, vmess
JSON with string ports, ss legacy + SIP002 + plugin, hy2 with obfs + port hopping, tuic, wg,
socks/http with auth, IPv6, emoji remarks, Russian remarks).

## Done
    scripts/ru1 cargo fmt --all --check
    scripts/ru1 cargo clippy -p flamix-core --all-targets -- -D warnings
    scripts/ru1 cargo test -p flamix-core
(Other modules are `todo!()` stubs — that's fine, only your tests must pass.)
Commit on `oc/T01` with a meaningful message. Report: branch, commit hash, test count, anything unclear.

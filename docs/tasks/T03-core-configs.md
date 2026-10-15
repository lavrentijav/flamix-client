# T03 — Xray / sing-box config generation

Read `AGENTS.md` first (build rule: everything via `scripts/ru1`). Contract: `crates/flamix-core/src/model.rs`,
`crates/flamix-core/src/config/mod.rs` (module docs explain the two-core design).

## Setup
    git -C /srv/dev/projects/flamix-client worktree add /srv/dev/projects/flamix-client-wt/T03 -b oc/T03
    cd /srv/dev/projects/flamix-client-wt/T03

## Files you own
`crates/flamix-core/src/config/{mod.rs,xray.rs,singbox.rs}` (+ new submodules under `config/`, e.g. `routing.rs`).
Build `Server` values by hand in tests (the link parser is another task).

## Xray (`xray.rs`) — current Xray-core (v25/v26) JSON
- `outbound`: vless (users[0]: id, encryption "none", flow), vmess (id, alterId, security), trojan,
  shadowsocks (method, password; plugins unsupported -> still emit, log nothing), socks/http (with auth),
  wireguard (secretKey, address, peers[publicKey, preSharedKey, endpoint, allowedIPs 0.0.0.0/0 ::/0],
  reserved, mtu). `streamSettings`: network (`raw` for tcp is fine, `xhttp`, `ws`, `grpc`,
  `httpupgrade`, `h2`→`http`... use the names current Xray accepts; tcp with header http ->
  `rawSettings.header`), security tls (`serverName`, `alpn`, `fingerprint` = link fp or
  `settings.default_fingerprint`, `allowInsecure`) / reality (`serverName`, `publicKey`, `shortId`,
  `spiderX`, `fingerprint`). xhttp: `xhttpSettings {path, host, mode, extra}`. grpc: `serviceName`,
  `multiMode` when mode == "multi". `mux` when `settings.mux` and no flow. Tag "proxy".
- `full_config`: `log.loglevel` = settings.log_level; inbounds: `socks` (tag "socks", udp true,
  sniffing http/tls/quic routeOnly) on socks_port and `http` (tag "http") on http_port, listen
  127.0.0.1 or 0.0.0.0 if `allow_lan`; `dokodemo-door` tag "api" on 127.0.0.1:api_port for the API;
  `api {tag:"api", services:["StatsService"]}`, `stats {}`, `policy.system` uplink/downlink stats on
  (inbound + outbound); outbounds: proxy, `freedom` tag "direct", `blackhole` tag "block";
  dns: servers [settings.dns.remote, {address: settings.dns.direct, domains: [geosite:category-ru] when preset BypassRu}];
  routing: domainStrategy from settings, rules in order: api inbound -> api; custom rules (domain/ip/
  port/network; processes are ignored in Xray) -> their action tag; `block_quic` -> udp 443 block;
  preset rules (see below).
## sing-box (`singbox.rs`) — sing-box **1.12+** format (new DNS server format `{type, server, tag}`,
`route.rule_set` remote `.srs` instead of removed geoip/geosite, `action: "route"/"reject"`,
no legacy special outbounds `block`/`dns`: use rule actions `reject`, `hijack-dns`, `sniff`).
- `outbound`: vless/vmess/trojan/shadowsocks/hysteria2 (password, obfs salamander, `server_ports`
  from port hopping range "a-b" -> ["a:b"], up/down mbps)/tuic/wireguard (as `endpoints` entry in
  1.12: return the endpoint object; document it)/socks/http. tls {enabled, server_name, alpn,
  insecure, utls {enabled, fingerprint}, reality {enabled, public_key, short_id}}. transports: ws
  (path, headers.Host), grpc (service_name), httpupgrade (host, path), h2 → `http`. xhttp and kcp ->
  return `None` (sing-box has no xhttp).
- `full_config`: inbound `mixed` on socks_port (Proxy mode) or `tun` (TUN mode: `address`
  ["172.19.0.1/30"] (+ "fdfe:dcba:9876::1/126" if tun_ipv6), mtu, stack, auto_route, strict_route,
  `route_exclude_address` private ranges) ; outbound "proxy" = `outbound(...)` or, if
  `upstream_socks` is Some(port), a socks outbound to 127.0.0.1:port; outbound "direct";
  route: `auto_detect_interface: true`, rules: `{action: sniff}`, `{protocol: dns, action: hijack-dns}`,
  in TUN mode `{process_name: ["xray", "xray.exe", "sing-box", "sing-box.exe"], outbound: direct}` (loop prevention for
  the chained Xray), private IPs direct (`ip_is_private: true`), custom rules (domain -> `domain_suffix`/
  `domain`/`domain_regex`/`rule_set` mapping from Xray syntax; geosite:X -> remote rule set
  `geosite-X` from `https://raw.githubusercontent.com/SagerNet/sing-geosite/rule-set/geosite-X.srs`,
  geoip:X -> `https://raw.githubusercontent.com/SagerNet/sing-geoip/rule-set/geoip-X.srs`,
  `download_detour: "proxy"`; processes -> `process_name`), then preset rules, `final: "proxy"`.
  dns: servers remote (tag "remote", detour proxy) / direct (tag "direct"), rule: rule-set geosite-category-ru
  -> direct when BypassRu; `fakeip` when `settings.dns.fake_ip` in TUN mode.
  `experimental.clash_api.external_controller = "127.0.0.1:<api_port>"`. `log.level` mapping
  (Xray "warning" -> sing-box "warn").
## Presets (shared helper, both cores)
- Global: private -> direct; rest -> proxy.
- BypassRu: `geosite:category-ads-all` -> block; `geosite:private`, `geoip:private`,
  `geosite:category-ru`, `geoip:ru`, `domain:ru`, `domain:su`, `domain:xn--p1ai` -> direct; rest -> proxy.
- Custom: only custom rules; rest -> proxy.

## `plan` (mod.rs)
As documented. Proxy mode: Xray if supported else sing-box (mixed inbound). TUN: sing-box elevated;
if `singbox::outbound` returns None, also an Xray plan (not elevated) and sing-box gets
`upstream_socks = Some(settings.socks_port)`; in that case sing-box must NOT bind the mixed inbound.
Xray plans come before sing-box. `api_port` via `config::api_port`.

## Tests (required)
Snapshot-style assertions with `serde_json::json!` fragments (check key paths, not whole JSON) for:
vless reality vision, vless xhttp + extra, vmess ws tls, trojan grpc, ss, hy2 hopping (sing-box),
tuic, wireguard; presets in both cores; TUN chain plan for xhttp; Proxy plan for hy2 = sing-box only.
Optionally (nice): if env `FLAMIX_XRAY_BIN` / `FLAMIX_SINGBOX_BIN` is set, write configs to a temp dir and
run `xray run -test -c` / `sing-box check -c` on them (ignored when unset).

## Done
    scripts/ru1 cargo fmt --all --check
    scripts/ru1 cargo clippy -p flamix-core --all-targets -- -D warnings
    scripts/ru1 cargo test -p flamix-core config
Commit on `oc/T03`. Report: branch, commit, test results, any format doubts.

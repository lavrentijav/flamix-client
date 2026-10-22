# Transparent proxy + AP tether (Linux, root)

This doc describes two "root" modes on Linux where the client captures traffic
without exposing a VPN-like tunnel device to applications.

## Why

TUN mode works, but it is visible: `ip link` shows a `tun0`/`utun`, apps and
sandboxes can detect the tunnel, and some sites block traffic that originates
from a known VPN interface. We also want to send traffic from the device's
own access-point / USB-tether into the tunnel, which TUN does not do by itself.

Two features cover this:

1. **TProxy mode** — transparent proxy via `nftables TPROXY`. No tun device,
   no proxy env var; applications see the real routes, the real DNS, the real
   interfaces. Netfilter diverts their packets to a local port the core binds
   with `IP_TRANSPARENT`.
2. **AP tether** — route traffic *forwarded through* the device (from an
   access-point interface such as `wlan0` running hostapd, or `usb0` from USB
   tethering) into the tunnel. Works with both TProxy and TUN mode.

Both require root — the netfilter rules, the policy routing table, and
`cap_net_admin` on the core socket are privileged operations.

## Platforms

- **Linux**: supported — nftables + policy routing.
- **Windows**: use TUN mode. Transparent capture would need a WFP filter driver,
  out of scope.
- **macOS**: use TUN mode (`pf` has no TPROXY equivalent).
- **Android**: VpnService is the only reliable path; a rooted device could run
  the same nftables rules, but that is not a target.

## Data model

New fields in `Settings` (`crates/flamix-core/src/model.rs`), all `#[serde(default)]`:

- `mode: ConnectionMode::TProxy` — new variant alongside `Proxy` and `Tun`.
- `tproxy_port: u16` — local port the core binds for TPROXY (default `12345`).
- `tproxy_mark: u32` — fwmark used in `ip rule` lookup (default `0x1a2b`).
- `tproxy_table: u32` — routing table id for the fwmark lookup (default `100`).
- `tproxy_route_exclude: Vec<String>` — CIDRs that must bypass the tunnel.
  Baked-in defaults always apply (RFC1918, loopback, link-local, multicast,
  carrier-grade NAT, broadcast); the user list is additive.
- `ap_tether_enabled: bool` — redirect traffic forwarded from AP interfaces
  into the tunnel (default `false`).
- `ap_interfaces: Vec<String>` — interface names to treat as AP input (e.g.
  `["wlan0", "usb0"]`). Empty + `ap_tether_enabled` is a no-op.

Serde stability is preserved: existing state files miss these keys and fall
back to defaults, old `mode` strings still parse.

## sing-box config shape

When `mode = TProxy`, the generator emits a `tproxy` inbound in place of the
`tun` / `mixed` inbound:

```json
{
  "type": "tproxy",
  "tag": "tproxy-in",
  "listen": "127.0.0.1",
  "listen_port": 12345,
  "network": "tcp udp",
  "sniff": true
}
```

The rest of the config (route, outbounds, DNS) is identical to proxy mode.
Core override: `tproxy` is a sing-box feature only; Xray-core does not have an
equivalent inbound, so TProxy mode forces `sing-box` as the core.

## Netfilter + routing (flamix-sys)

Module: `crates/flamix-sys/src/transparent.rs`, Linux-only.

### Policy routing

```
ip rule add fwmark <mark>/<mark> lookup <table> priority 100
ip route add local 0.0.0.0/0 dev lo table <table>
# IPv6 equivalents if sing-box tproxy IPv6 inbound is enabled (future).
```

The `local` route is what makes TPROXY work: the kernel treats the diverted
packets as locally-destined, delivering them to the TPROXY socket on `lo`.

### nftables ruleset

One table `flamix` with the critical chains in the `mangle` family (type
`route` and `filter` respectively). Pseudo-code:

```
table inet flamix {
  set bypass_v4   { type ipv4_addr; flags interval; elements = { <baked-in CIDRs>, <user CIDRs> } }
  set ap_ifaces   { type ifname; elements = { <user AP interfaces> } }

  chain divert {           # match-and-mark side effect
    type filter hook prerouting priority mangle; policy accept;
    meta l4proto { tcp, udp } socket transparent 1 meta mark set <mark> accept
  }

  chain prerouting {
    type filter hook prerouting priority mangle; policy accept;
    jump divert
    # AP-tether path: anything forwarded through an AP input interface →
    # TPROXY to the local core.
    iifname @ap_ifaces ip daddr @bypass_v4 accept
    iifname @ap_ifaces meta l4proto tcp tproxy to :<port> meta mark set <mark> accept
    iifname @ap_ifaces meta l4proto udp tproxy to :<port> meta mark set <mark> accept
  }

  chain output {
    type route hook output priority mangle; policy accept;
    # Loop prevention: skip the core's own packets.
    meta skuid <core-uid> accept
    # Baked-in and user bypass.
    ip daddr @bypass_v4 accept
    meta l4proto { tcp, udp } meta mark set <mark>
  }
}
```

Reroute of locally-generated traffic uses `mangle OUTPUT` + a second trip
through the stack (`meta mark set` plus `type route` chain triggers a route
re-eval). The AP-tether path uses `tproxy to :port` on PREROUTING directly,
since those packets are forwarded, not locally generated.

### Loop prevention

The core must *not* match its own egress traffic, or we build an infinite
loop. The ruleset excludes by `skuid` — sing-box is launched under a fixed
service uid (we create `flamix-tproxy` on install; fallback is `nobody`).

### Install / remove

- `install(settings) -> Result<InstalledTproxy>`: writes an nft fragment to
  stdin of `nft -f -`, runs `ip rule add`, `ip route add`. All operations are
  idempotent (we flush our table and remove our rule by priority first).
- `remove()`: `nft delete table inet flamix`, `ip rule del`, `ip route del`.
- Teardown must run from the same elevated watchdog that runs the core
  (see `runner.rs`), so a crashed app still cleans up.

## Runner integration

The same elevated-child watchdog pattern as TUN is reused: an auto-generated
POSIX shell helper runs as root, installs the ruleset, starts the core as the
service uid (via `setpriv`), monitors the app pid, and tears everything down
on exit. One shell script, no systemd dependency.

## Rollback / failure

Any error in `install` triggers `remove` before returning. On startup the
module also removes a stale `flamix` table and `ip rule/route` from a previous
unclean shutdown, so the user is never stuck without network after a crash.

## Milestones

- **M1**: model + sing-box tproxy inbound + `transparent.rs` with install/remove
  (unit tests on the generated nft text) + user doc.
- **M2**: runner integration (elevated watchdog running tproxy mode) + ru1
  smoke test (fire up a server, point client at it, curl through TPROXY).
- **M3**: UI panel for the new mode and AP interface list.
- **M4**: IPv6 TPROXY and nftables-only hosts (iptables fallback dropped on
  purpose).

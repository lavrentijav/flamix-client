# Failover & multi-server ("резервные серверы", "мультитрафик")

Status: **planned** (design by the lead; tasks T12–T14). Owner of this doc: lead.

## What the user gets
1. The user picks a **primary** server and an ordered list of **backups** (any servers, from any subscriptions).
2. The primary carries traffic. Backups are kept **warm**: health-probed every 2 s (configurable), their transport
   pre-connected where the protocol allows (mux / xhttp keep-alive), so switching costs no handshake.
3. When the primary dies, **new connections go to the first healthy backup instantly** (target: < 1 s detection,
   0 s switch). When it comes back, traffic returns after a hold-down period (default 60 s, no flapping).
4. **Same exit IP group**: servers whose public exit IP is identical (measured, see below) are marked as a group. Switching
   inside such a group is the "seamless" case — websites, banks, games and IP-bound sessions don't see an IP change.
5. **Multi-traffic** (optional modes beyond failover): `load-balance` (spread new connections across healthy servers,
   least-latency or round-robin) and per-rule server choice (routing rule -> a specific server, e.g. streaming via NL,
   everything else via DE).

## What is physically possible (be honest in the UI)
- An existing **TCP** connection lives on the exit server; when that server dies, the stream is gone. No client can
  migrate it. What we *can* do:
  - **freeze instead of fail** — our local gateway (below) holds *new* connection attempts during the switch
    (up to `freeze_ms`, default 3000) and dials them via the backup, so apps see a slow connect, not an error;
  - **early-failure retry** — if a connection fails before any byte reached the app, it is transparently re-dialled via the
    backup (safe: nothing was delivered yet);
  - apps reconnect by themselves; with the **same exit IP** their sessions/cookies/IP-bound tokens stay valid.
- **UDP** flows (QUIC/HTTP3, games, calls) are stateless for us: the gateway re-pins the flow to the backup. With the
  **same exit IP**, QUIC servers treat the new source port as NAT rebinding → **the connection survives without a drop**.
  This is the "без обрыва, пробить" case and the main reason to measure exit IPs.

## Architecture
```
apps ──► flamix-gate (Rust, local SOCKS5/HTTP mixed inbound + UDP) ──► per-server local SOCKS inbounds
                                                                      in ONE core process (xray / sing-box):
                                                                      inbound s1 -> outbound server1
                                                                      inbound s2 -> outbound server2 ...
TUN mode: sing-box tun ──► flamix-gate ──► (same)
```
- **One core process** with N outbounds (primary + backups), each reachable through its own local SOCKS inbound
  (`127.0.0.1:<port_i>`), so switching never restarts the core.
- **flamix-gate** (new crate `crates/flamix-gate`, pure Rust/tokio): owns the decision. Per new TCP connection: pick
  the active server, dial through its SOCKS inbound; on dial failure or failure-before-first-byte → retry on the next
  healthy server; while no server is healthy → freeze (queue) up to `freeze_ms`. UDP: SOCKS5 UDP ASSOCIATE per flow,
  flow table re-pinned on switch. Exposes stats per server (bytes, active connections) — also feeds per-server traffic.
- **Health prober** (in gate): every `probe_interval` per server: HTTP `GET` of `settings.ping_url` through that server's
  inbound (real end-to-end check, ~1 RTT on a warm connection); 2 consecutive failures = dead (primary also marked dead
  immediately on N dial errors in a row from real traffic — passive detection is faster than probing).
- **Exit IP probe**: through each server fetch `https://api.ipify.org` (fallbacks: `https://ifconfig.me/ip`,
  `https://icanhazip.com`) on connect and every 10 min; servers with equal exit IPs form a group; UI shows the group badge
  and prefers same-group backups first.
- **Without the gate** (fallback / simple mode): Xray `observatory` + `routing.balancers` (strategy `leastPing`,
  `fallbackTag`) and sing-box `urltest` outbound give core-level failover for new connections, no freeze, no UDP re-pin.
  Used if the gate is disabled in settings.

## Model additions (to add when T12 starts)
```rust
pub struct Failover {
    pub enabled: bool,
    pub mode: FailoverMode,          // Failover | LoadBalance
    pub backups: Vec<String>,        // server ids, ordered
    pub probe_interval_ms: u32,      // 2000
    pub freeze_ms: u32,              // 3000
    pub hold_down_s: u32,            // 60
    pub prefer_same_exit_ip: bool,   // true
    pub use_gate: bool,              // true; false = core balancer only
}
// Server: + exit_ip: Option<String>, exit_ip_checked: Option<Timestamp>
// RoutingRule: + server: Option<String>  (per-rule server for multi-traffic)
```

## Tasks
- **T12** core: model additions; Xray multi-inbound/multi-outbound config + balancer/observatory fallback; sing-box
  `urltest`; per-rule server outbounds. (after T03)
- **T13** `crates/flamix-gate`: mixed inbound, dial-through-SOCKS, health prober, freeze queue, early-failure retry, UDP
  re-pinning, exit-IP probe, stats; heavy test suite with fake upstream SOCKS servers that die on command. (after T12)
- **T14** app: backup list editor (drag to reorder, "add as backup" in server row menu), group badges (same exit IP),
  live state (active / warm / dead) on the connection panel. (after T07, T13)

# Flamix

Cross-platform VPN/proxy client written in Rust, with a Slint UI (WinUI 3 / Fluent look).

[![CI](https://github.com/lavrentijav/flamix-client/actions/workflows/ci.yml/badge.svg)](https://github.com/lavrentijav/flamix-client/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/lavrentijav/flamix-client?include_prereleases&label=release)](https://github.com/lavrentijav/flamix-client/releases)
[![License](https://img.shields.io/badge/license-GPL--3.0--or--later-blue)](LICENSE)
[![Platforms](https://img.shields.io/badge/platforms-Windows%20%7C%20macOS%20%7C%20Linux-informational)](#platforms)

**[🇷🇺 Русский](README.md) | 🇬🇧 English**

> **Status: early development (alpha).** Features and UI change often; builds will appear on the
> [Releases](https://github.com/lavrentijav/flamix-client/releases) page once CI is ready.

Flamix is an open client that runs the external **Xray-core** and **sing-box** cores (like v2rayN) and
brings the features of Happ, v2RayTun and v2rayN into one application.

## Screenshots

Dark theme, all tabs (taken with demo data).

| Servers | Add |
|---|---|
| ![Servers](docs/screenshots/servers.png) | ![Add](docs/screenshots/add.png) |
| **Routing** | **Settings** |
| ![Routing](docs/screenshots/routing.png) | ![Settings](docs/screenshots/settings.png) |
| **Statistics** | **About** |
| ![Statistics](docs/screenshots/stats.png) | ![About](docs/screenshots/about.png) |

## Features

The window has a tab rail on the left, the server list in the middle and the connection panel on the right.

- **Servers** — subscriptions that can be collapsed and pinned, traffic bars for the subscription and for single
  servers, latency tests (TCP and through the core), search, sorting by ping, exit IP check, update all subscriptions.
- **Add** — subscription by URL (with a name and backup addresses), a list of share links (`vless://`, `vmess://`,
  `trojan://`, `ss://`, `hy2://` and others, one per line), import from the clipboard.
- **Routing** — presets "Global", "RU bypass" and "Custom rules"; a switch for the Anixart mirror (direct or via the
  proxy); your own rules by domains, IPs and processes with the actions "proxy", "direct", "block"; routing pushed
  by the provider.
- **Settings** — Proxy, TUN or **TProxy** mode (Linux-only transparent proxy via nftables — no visible `tun0`,
  applications cannot tell they are being filtered) and **AP tether** (traffic forwarded through `wlan0`/`usb0`
  goes into the tunnel); core choice (Auto, Xray, sing-box, mihomo), ports, LAN access, system proxy, UDP,
  sniffing, TUN parameters, DNS, the ping test address, geo data, autostart, theme.
- **Statistics** — traffic for the last 30 days and the most used servers.
- **About** — version, core versions, licenses, backup of settings and subscriptions.

**Protocols:** VLESS (Reality, XTLS Vision, XHTTP), VMess, Trojan, Shadowsocks (SIP002, plugins),
Hysteria2 (port hopping), TUIC, WireGuard, SOCKS, HTTP.
**Transports:** TCP/RAW, WS, gRPC, HTTPUpgrade, XHTTP, H2, mKCP, QUIC.

Cores: **Xray-core** for most protocols, **sing-box** for Hysteria2 and TUIC, **mihomo** as an optional core.
"Auto" picks the core for each server's protocol.

## Comparison

| Feature | Flamix | Happ | v2RayTun | v2rayN |
|---|:---:|:---:|:---:|:---:|
| Open source | ✅ | ❌ | ❌ | ✅ |
| Desktop: Windows / macOS / Linux | ✅ | ✅ | ? | ✅ |
| TUN mode | ✅ | ✅ | ? | ✅ |
| Backup subscription URLs | ✅ | ✅ | ❌ | ❌ |
| Per-server traffic | ✅ | ❌ | ❌ | ❌ |
| Backup servers and multi-traffic | 🚧 | ❌ | ❌ | ✅ |
| Xray-core | ✅ | ✅ | ? | ✅ |
| sing-box | ✅ | ✅ | ? | ✅ |
| Understands other clients' deep links | ✅ | ❌ | ? | ? |

"?" — no data, so we do not guess. Based on public documentation as of October 2026.

## Platforms

- **Windows** 10/11 x64
- **macOS** 11+ (universal)
- **Linux** x86_64 / aarch64: Debian/Ubuntu (`.deb`), Fedora/RHEL (`.rpm`), Arch (PKGBUILD),
  any distro (AppImage, `.tar.gz`)
- **Mobile** (Android, iOS) — planned

## Download and install

Ready-made builds are published on the [Releases](https://github.com/lavrentijav/flamix-client/releases) page.

Step-by-step instructions per platform (deb, rpm, PKGBUILD, AppImage, `tar.gz`, macOS Gatekeeper, etc.) —
see **[docs/install.md](docs/install.md)**.

## Quick start

1. **Add a subscription.** Open the "Add" tab and paste the subscription URL. You can also copy a link
   (`flamix://…`, `happ://…`, `v2raytun://…`, `vless://…` and others) and press "From clipboard".
2. **Pick a server** in the list. The "Ping test" button shows its latency.
3. **Connect** with the power button on the right. **Proxy** mode sets the system proxy and opens local
   SOCKS5 `10808` and HTTP `10809`. **TUN** mode routes all system traffic and needs administrator or root rights.
   **TProxy** mode (Linux, root) is a netfilter-based transparent proxy: applications see no `tun0`, no
   `HTTPS_PROXY`, just the real network with its direct route removed and the proxy inserted. An optional
   **AP tether** forwards traffic from interfaces like `wlan0` (hostapd) or `usb0` (USB tethering) through the
   same tunnel. Details — [docs/transparent-proxy.md](docs/transparent-proxy.md).
4. **Choose routing** on the "Routing" tab. "RU bypass" is usually the right choice for Russian sites.
5. **Update the subscription** with the button on its card. If the main address fails, the client tries the backups.

If a server does not connect, try another core on the "Settings" tab → "Core". TUN on Windows needs administrator
rights; the client offers to restart with them.

## For providers

Flamix reads subscription headers the same way Happ and v2RayTun do, and adds a few features of its own.
The full list and formats are in **[docs/subscription-headers.md](docs/subscription-headers.md)**. In short:

| Header | What the client does with it |
|---|---|
| `profile-title` | subscription name (`base64:` allowed) |
| `subscription-userinfo` | subscription traffic and expiry |
| `profile-update-interval` | auto-update interval in hours |
| `support-url` | support button; `t.me` and `tg://` links get the Telegram icon, `mailto:` gets an envelope |
| `profile-web-page-url` | website button; shown only when the header is set |
| `announce` | announcement text above the servers |
| `sub-info-text`, `sub-info-color`, `sub-info-button-text`, `sub-info-button-link` | banner: text, colour (`red`, `blue`, `green`), button |
| `sub-expire`, `sub-expire-button-link` | expiry notice (replaces the banner) |
| `profile-backup-urls` (aliases `fallback-url`, `fallback-urls`, `backup-urls`) | backup subscription addresses |
| `server-userinfo` and the `userinfo=` link parameter | traffic and expiry per server |
| `providerid` | provider id (header, `#providerid X` in the body, or URL fragment) |
| `update-always` | update the subscription on every app start |
| `network-filter` | keep only servers with the given transport or protocol |
| `subscription-refill-date` | traffic reset date |
| `routing`, `routing-enable` | provider routing (`happ://routing/…` link or base64 JSON) |
| `new-url`, `new-domain` | replace the subscription address |
| `x-hwid-limit`, `x-hwid-active`, `x-hwid-max-devices-reached`, `x-hwid-not-supported` | device limit from the panel's response |
| `socks-auth-*`, `http-auth-*` | authentication on the client's local ports |

When requesting a subscription the client sends an `x-hwid` header and a `User-Agent: Flamix/…` header.
Keys the client does not use yet are stored and do not break subscription parsing.

Example of per-server traffic in a link:

```
vless://uuid@de1.example.com:443?type=ws&security=tls&userinfo=download%3D5368709120%3Btotal%3D53687091200#🇩🇪 Germany
```

## Build from source

You need [Rust](https://rustup.rs) (stable, via rustup) and the system dependencies for your OS. In short:

```
cargo build --release -p flamix
cargo test --workspace
```

Detailed instructions (per-platform dependencies, packaging) — see **[docs/build.md](docs/build.md)**.

## Project structure

```
crates/flamix-core   # platform-independent logic: model, links, subscriptions, configs, store, ping
crates/flamix-sys    # OS integration: system proxy, core processes, elevation, autostart
app/flamix           # Slint desktop app (ui/*.slint, src/*.rs)
docs/                # specs and design
scripts/             # build and packaging scripts
```

## Roadmap

- [ ] 🚧 Core logic: model, links, subscriptions, configs, store, ping — in progress
- [ ] 🚧 OS integration: system proxy, core processes, elevation, autostart — in progress
- [ ] 🚧 Slint main window, themes, RU/EN — in progress
- [ ] 🚧 Packaging and CI: deb, rpm, PKGBUILD, AppImage, tar.gz, Windows installer, macOS `.dmg` — in progress
- [ ] Backup servers and multi-traffic — planned ([docs/failover.md](docs/failover.md))
- [ ] Android — planned
- [ ] iOS — planned

## Credits

- [Xray-core](https://github.com/XTLS/Xray-core) — proxy core
- [sing-box](https://github.com/SagerNet/sing-box) — proxy core
- [Slint](https://slint.dev) — UI framework
- [Alfa-Bank icons](https://github.com/core-ds/icons) (`@alfalab/icons-glyph`, MIT) — icon set;
  license text in [app/flamix/assets/icons/LICENSE-alfalab.txt](app/flamix/assets/icons/LICENSE-alfalab.txt)

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md). Questions and bugs — in
[Issues](https://github.com/lavrentijav/flamix-client/issues).

## License

**GPL-3.0-or-later**. Full text in the [LICENSE](LICENSE) file.

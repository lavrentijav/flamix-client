# T10 — README (RU + EN), LICENSE, repo presentation

Read `AGENTS.md` first. No Rust build needed.

## Setup
    git -C /srv/dev/projects/flamix-client worktree add /srv/dev/projects/flamix-client-wt/T10 -b oc/T10
    cd /srv/dev/projects/flamix-client-wt/T10

## Files you own
`README.md` (Russian, main), `README.en.md` (English twin), `LICENSE`, `docs/README-assets/**` (if needed),
`CONTRIBUTING.md`, `.github/ISSUE_TEMPLATE/**` (bug report + feature request, Russian).
Do NOT write install/build instructions in detail: task T09 writes `docs/install.md` and `docs/build.md` —
link to them (they may not exist yet in your branch; link anyway).

## Facts (use only these, do not invent features or URLs)
- Name: **Flamix** (repo `lavrentijav/flamix-client`, https://github.com/lavrentijav/flamix-client), license **GPL-3.0-or-later**.
- What: cross-platform VPN/proxy client written in Rust, UI on Slint (WinUI 3 / Fluent look), runs external cores
  **Xray-core** (https://github.com/XTLS/Xray-core) and **sing-box** (https://github.com/SagerNet/sing-box) like v2rayN.
  Goal: the features of Happ, v2RayTun and v2rayN in one open client.
- Platforms: Windows 10/11 x64, macOS 11+ (universal), Linux x86_64/aarch64 — Debian/Ubuntu (.deb), Fedora/RHEL (.rpm),
  Arch (PKGBUILD), any distro (AppImage, tar.gz). Mobile (Android, iOS): planned.
- Protocols: VLESS (Reality, XTLS Vision, XHTTP), VMess, Trojan, Shadowsocks (SIP002, plugins), Hysteria2 (port hopping),
  TUIC, WireGuard, SOCKS, HTTP. Transports: TCP/RAW, WS, gRPC, HTTPUpgrade, XHTTP, H2, mKCP, QUIC.
- Modes: Proxy (system proxy, local SOCKS5 10808 / HTTP 10809) and TUN (whole-system, admin/root).
- Subscriptions: base64 / plain / Xray & sing-box JSON / Clash YAML; headers `subscription-userinfo`, `profile-title`,
  `profile-update-interval`, `support-url`, `profile-web-page-url`, `announce`; HWID headers (Remnawave-compatible);
  **backup subscription URLs** (`profile-backup-urls`) and **per-server remaining traffic** (`server-userinfo` header or
  `userinfo=` link parameter) — full spec in `docs/subscription-headers.md` (summarise it with a short example for
  panel/provider authors).
- Deep links: `flamix://import/<url>`; also understands `happ://add/…`, `v2raytun://import/…`, `v2rayng://install-sub?url=…`,
  `clash://install-config?url=…`, `sing-box://import-remote-profile?url=…`; import from clipboard.
- Routing presets: Global, "Bypass RU" (RU sites/IPs direct, ads blocked), Custom rules (domains, IPs, processes in TUN).
- Ping: TCP and real URL test through the core; per-subscription traffic bar, per-server traffic bar, statistics page,
  tray, autostart, light/dark theme, RU/EN UI.
- Icons: Alfa-Bank open icon set `@alfalab/icons-glyph` / `@alfalab/icons-flag` (MIT) — https://github.com/core-ds/icons ,
  license text in `app/flamix/assets/icons/LICENSE-alfalab.txt`.
- Status: **early development (alpha)** — say so clearly at the top; releases appear on the Releases page when CI is ready.
- Screenshot: `docs/design/screenshot-main.png` (may not exist yet — reference it anyway, plus the mockup
  `docs/design/main-window.png` captioned as "design mockup").

## README structure
1. Title + one-line tagline, badges (shields.io): CI workflow status (`.github/workflows/ci.yml`), latest release,
   license, platforms. 2. Language switch "🇷🇺 Русский | 🇬🇧 English". 3. Screenshot. 4. Features (grouped, concise).
5. Comparison table Flamix vs Happ vs v2RayTun vs v2rayN (rows: open source, desktop OSes, TUN, backup subscription URLs,
   per-server traffic, Xray, sing-box, deep links of others) — only mark what's true per the facts above; for other clients
   use "?" where you are not sure instead of guessing. 6. Download / install → link `docs/install.md` + Releases page.
7. Quick start (add subscription, choose server, connect, Proxy vs TUN). 8. For providers: subscription headers summary +
   link to spec. 9. Build from source → `docs/build.md`. 10. Project structure (crates/flamix-core, crates/flamix-sys,
   app/flamix, docs). 11. Roadmap checklist (desktop core ✅/🚧, packaging, Android, iOS). 12. Credits (Xray-core, sing-box,
   Slint, Alfa-Bank icons). 13. License.
Keep it scannable; no marketing fluff. `LICENSE` = official GPL-3.0 text (download from https://www.gnu.org/licenses/gpl-3.0.txt).

## Checks
All relative links point to paths that exist in main or are listed above as upcoming (`docs/install.md`, `docs/build.md`,
`docs/design/screenshot-main.png`). Markdown renders (no broken tables).

## Done
Commit on `oc/T10`. Report: branch, commit hash, list of links used.

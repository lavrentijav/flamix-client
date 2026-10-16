# T22 — research & design: Android port (feasibility + plan)

Read `AGENTS.md` first. **No Rust build, no code.** Output is a design/feasibility document the lead turns
into tasks. Precision and sources matter more than volume; mark anything unverified as **unverified**.

## Setup
    git -C /srv/dev/projects/flamix-client worktree add /srv/dev/projects/flamix-client-wt/T22 -b oc/T22
    cd /srv/dev/projects/flamix-client-wt/T22

## Files you own
`docs/research/android.md` (new). Nothing else. (Do NOT touch `~/doc` — that is the lead's.)

## Context to read first
- `README.md`, `docs/architecture.md`, `docs/research/feature-matrix.md`, `docs/research/gaps.md`.
- The tree: `crates/flamix-core` (platform-independent logic: model, links, subscriptions, configs,
  store), `crates/flamix-sys` (OS integration: system proxy, core processes, elevation, autostart —
  desktop/Unix oriented), `app/flamix` (Slint UI + controller + tray).
- `docs/failover.md` (planned `flamix-gate` crate) — relevant because mobile needs a similar
  per-connection layer.

## Allowed network access (read-only fetches / `git clone --depth 1` into `/srv/dev/tmp/flamix-android-research/`)
- Slint: <https://slint.dev> docs, <https://github.com/slint-ui/slint> (Android/mobile backend, `slint-build`,
  `android-activity` / `ndk` integration, examples under `examples/`)
- Xray-core and sing-box Android builds: their GitHub releases / gomobile `.aar` docs
- Android `VpnService` (developer.android.com), `android-activity` crate, `jni` crate, `cargo-ndk`
- v2rayNG / Hiddify / Happ Android (open repos only) for how they do VpnService + core + UI
- Gradle/NDK packaging, APK/AAB, GitHub Actions Android
Nothing else; no logins, no POST, no installing apps.

## The document `docs/research/android.md` must answer
1. **Reuse boundary.** Exactly which parts of `flamix-core` are already platform-independent and reusable
   as-is on Android (`aarch64-linux-android`), which need `cfg`-gating, and which belong only to desktop.
   Name the files/modules and any blockers (e.g. `directories` data dir, `reqwest`/rustls on Android,
   `chrono`, threads/tokio, filesystem paths, `flamix-sys` Unix calls).
2. **VPN on Android.** How traffic is captured on a non-root device: `VpnService` + a userspace core.
   Decide/compare the realistic options:
   (a) route Xray/sing-box as the VPN process via a gomobile/`libcore` + `VpnService.Builder`,
   (b) our own Android `VpnService` (Kotlin) that forwards into a local core (like desktop Proxy mode),
   (c) tun2socks-style. State what requires a `MainActivity`/VpnService **Kotlin/Java** glue (unavoidable
   on Android) and the minimal JNI surface.
3. **UI.** Can Slint target Android today (which Slint version, which backend, `android-activity`
   integration, input/touch, safe areas, DPI)? What Slint features we already use (`std-widgets`, fluent
   style, `femtovg`/`software` renderers, `slint-build`) work on Android, and what does not (tray, file
   dialogs `rfd`, `arboard`, `open`). Cite the Slint docs/examples.
4. **Architecture proposal.** A concrete crate/layout plan for Android (what to add: e.g. an
   `app/flamix-android` or `platform/android` glue crate, a Kotlin shell, JNI bridge; how the existing
   `controller.rs` and pages are reused vs. replaced; how the store path maps to Android app-private dirs).
   A short dependency/flow diagram (text).
5. **Build & CI.** How to build for `aarch64-linux-android` (+ `armv7`/`x86_64` emulator): `cargo-ndk`,
   NDK version, Gradle wrapper, signing, APK vs AAB, and a GitHub Actions job sketch. Note the core
   binaries (Xray/sing-box) — reuse upstream Android builds or build from source?
6. **Feature deltas vs desktop (honest).** What the Android UX needs that desktop does not: VPN permission
   flow, foreground service + persistent notification, per-app proxy (the one mobile-only killer feature
   from `gaps.md` #24), battery/doze, quick-settings tile, import via `ACTION_SEND`/`v2rayng://`
   schemes (see `docs/research/v2rayng.md`), no tray. What is out of scope for v1.
7. **Risks & unknowns.** Ranked list (e.g. Slint-mobile maturity, gomobile, Play Store policy for VPN
   apps, iOS later). For each: impact and a cheap way to de-risk.
8. **Suggested task breakdown** (small, verifiable; core → sys → glue → UI → packaging), with rough
   sizes S/M/L and dependencies, ending in a "first vertical slice" that proves the risky part early
   (e.g. a minimal app that opens a `VpnService` and pushes one config to an Xray core).

## Done
Commit `docs/research/android.md` on `oc/T22`. Every external claim gets a source URL; mark unverified.
Report: branch, commit, the 5 biggest risks, and your recommended first slice. Do not push, do not merge.

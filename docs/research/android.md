# Android port — feasibility & design (T22)

Status: **research/design**. No code was written for this task; this document is the
input for the lead to turn into tasks. Every external claim carries a source URL; claims I
could not verify are marked **unverified**. Local file references are `path:line`.

Scope: Android only. iOS is out of scope for v1 (see §7).

---

## 0. Executive summary

- **`flamix-core` is ~90 % reusable on Android.** The pure-logic modules (model, link parsers,
  subscription parsing, Xray/sing-box config generation, deeplink, country, provider routing,
  backup) contain no OS calls and compile for `aarch64-linux-android` as-is. The only concrete
  blockers are the data-directory lookup (`store::data_dir`), and the network layer
  (`reqwest`/`rustls`) which must be exercised under an active VPN (self-traffic loop).
- **`flamix-sys` is desktop-only.** Every module (system proxy, core processes, elevation,
  autostart, stats) shells out to desktop tools or uses POSIX/Windows process control.
- **Traffic capture must be a Kotlin `VpnService`.** Android has no userspace TUN without a
  `VpnService` (non-root). The realistic design is "Kotlin `VpnService` builds the TUN and hands
  the fd to an embedded Go core (sing-box `libbox` / Xray `libv2ray`)" — the same architecture
  v2rayNG and sing-box-for-android use. This is unavoidable: a small Kotlin/Java shell + a
  JNI surface is **required** on Android.
- **Slint does run on Android today** (Slint 1.x, `backend-android-activity-06`, min API 26),
  but the Android backend **only supports the Skia renderer**, requires the activity to be
  `android.app.NativeActivity` (or a subclass), and has no tray concept. `std-widgets` and the
  existing `.slint` pages should work; `rfd`, `arboard`, `tray-icon`, `ksni` do not.
- **Recommended first slice** (§8): a minimal APK that requests VPN permission, opens a TUN,
  pushes one hardcoded-but-`flamix-core`-generated VLESS config to an embedded sing-box
  `libbox`, and proves real browser traffic egresses through the tunnel while Flamix's own
  subscription fetch still works.

---

## 1. Reuse boundary

The workspace is a three-crate split (`Cargo.toml`): `crates/flamix-core`,
`crates/flamix-sys`, `app/flamix`. The Android target is `aarch64-linux-android`
(plus `armv7-linux-androideabi`, `x86_64-linux-android` for emulators; Slint docs list the
matching Rust targets — <https://github.com/slint-ui/slint/blob/master/docs/astro/src/content/docs/guide/platforms/mobile/android.mdx>).

### 1.1 `flamix-core` — reusable as-is (no `cfg` gating)

Pure computation / serde only, no `std::process`, no OS paths:

| Module | Notes |
|---|---|
| `model.rs` | Data contract only. |
| `link/` (`mod`, `vless`, `vmess`, `ss`, `hysteria2`, `tuic`, `socks_http`, `wireguard`) | Pure parsers/builders. |
| `subscription/` (`body`, `meta`, `util`) | Pure body/header parsing. |
| `deeplink.rs`, `country.rs`, `provider_routing.rs` | Pure. |
| `backup.rs` | serde-only; no filesystem access (`crates/flamix-core/src/backup.rs:1`). |
| `config/routing.rs`, `config/xray.rs`, `config/singbox.rs`, `config/mod.rs` | JSON generation only. `plan()`/`plan_cluster()` produce `CorePlan { core, config, elevated, api_port }` (`crates/flamix-core/src/config/mod.rs:168`). The `elevated` flag is meaningless on Android and should be ignored by the Android runner. |

The only `cfg` use in `flamix-core` is `#[cfg(test)]` (verified: `rg "cfg\\(" crates/flamix-core/src`
returns only test gates), so the whole crate is a plain library and will build for Android
without `cfg` scaffolding.

### 1.2 `flamix-core` — needs adaptation / `cfg` gating

| File:line | Issue | Fix |
|---|---|---|
| `store.rs:5-18` | `data_dir()` uses `directories::ProjectDirs::from("online","flamix","Flamix")`. Android has no such XDG/known-folder concept. | Already overridable with env `FLAMIX_HOME` (`store.rs:10-15`). Android shell sets `FLAMIX_HOME=<filesDir>/flamix` before `store::load`. Whether `directories` returns `None` or a bogus path on Android is **unverified**; the env override makes it moot. |
| `subscription/fetch.rs:3` | `reqwest` + `rustls-tls` (`Cargo.toml`) does DNS + TLS in-process. | Builds for Android in principle; must be tested on-device. `rustls`' default provider (`ring`/`aws-lc-rs`) on `aarch64-linux-android` is **unverified in this repo**. |
| `subscription/fetch.rs:301-336` | Device headers: `std::env::consts::OS` → `x-device-os`, plus `FLAMIX_OS_VERSION` / `FLAMIX_DEVICE_MODEL`. | Nice fit: on Android `std::env::consts::OS == "android"`; set the two env vars from Kotlin (`Build.VERSION.RELEASE`, `Build.MODEL`) so HWID headers are correct. |
| `ping.rs` | `tokio::net` TCP ping + `reqwest` URL ping through a local SOCKS. | Works, but under an active VPN the app's own sockets must be excluded from the tunnel (see §2.4). |
| `chrono` (`clock` feature) | `Local` timezone / wall clock. | `clock` pulls `iana-time-zone`/`libc`; Android should work but is **unverified**. `Utc::now()` (used everywhere) is fine regardless. |
| `config/singbox.rs:383` | TUN process rules emit `"process_name": ["xray", "xray.exe", …]`. | Android cannot match `process_name` (no procfs visibility); sing-box on Android supports `package_name` instead (<https://github.com/SagerNet/sing-box/blob/dev/docs/clients/android/features.md>). A generated `package_name` variant is needed for the per-app feature. |
| `config/mod.rs` `api_port` | Xray `socks_port+10`, sing-box `socks_port+11` (`config/mod.rs:145`). | Reusable; the clash API is reachable over loopback. |

### 1.3 `flamix-sys` — desktop-only (do not port; replace behind a trait)

All of `crates/flamix-sys` is OS integration and is explicitly documented as
"Mobile platforms do not use this crate" (`crates/flamix-sys/src/lib.rs:1`):

- `sysproxy.rs` — Windows registry, macOS `networksetup`, Linux `gsettings`/`kwriteconfig`
  (`sysproxy.rs:1-8`). Android has no system proxy to set; the VPN replaces it.
- `runner.rs` — spawns the core as a child process, plus a root-owned POSIX watchdog for
  elevated TUN (`runner.rs:1-8`). Android has no `fork/exec` of a core binary and no root.
- `elevation.rs` — polkit/`sudo`/UAC.
- `autostart.rs` — `.desktop` / LaunchAgents / registry Run key.
- `stats.rs` — Xray via `xray api statsquery` **CLI** (`stats.rs:1-20`); on Android there is no
  CLI, so only the sing-box clash-API path (`singbox_traffic`, `stats.rs:88`) is directly reusable.

### 1.4 `app/flamix` — desktop UI/glue

Reusable with care: `ui/*.slint`, `view.rs`, `pages/*` (model binding), and the pure parts of
`controller.rs` (state mutation, config generation, ping orchestration). Not portable:
`main.rs` (winit event loop + tray + single-instance TCP listener), `tray/*`,
`instance.rs`, `demo.rs` (`--screenshot` PNG), and the `rfd`/`arboard` deps in
`app/flamix/Cargo.toml`.

The Slint dependency is feature-gated for Windows/Linux/macOS today:
`app/flamix/Cargo.toml` enables `backend-winit`, `renderer-femtovg`, `renderer-software`.
On Android the winit backend is wrong and Slint only accepts `backend-android-activity-06`
with the **Skia** renderer (§3). Two ways to handle this are in §4.

### 1.5 Blockers summary

1. `directories` data dir → solved by `FLAMIX_HOME`.
2. `reqwest`+`rustls` on Android → build risk **unverified**; runtime loop risk real.
3. No `std::process` core launch → replace with embedded Go core (§2).
4. Slint desktop renderers/backends + `winit`/`rfd`/`arboard`/tray → cfg-gate for Android.
5. Process-based routing rules → need `package_name` variant for Android.

---

## 2. VPN on Android (non-root)

### 2.1 The only non-root mechanism is `VpnService`

Android exposes userspace TUN only through `android.net.VpnService`. The app calls
`VpnService.prepare()` to get user consent, then `VpnService.Builder`
(`addAddress`, `addRoute`, `addDnsServer`, `setMtu`, per-app `addAllowedApplication`/
`addDisallowedApplication`, `establish()`) to create the interface; the returned
`ParcelFileDescriptor` is the packet stream. The app must call `protect(fd)` on its own tunnel
socket to avoid a routing loop, and `onRevoke()` is called when the user/system disconnects.
Source: <https://developer.android.com/develop/connectivity/vpn>. There is exactly one active
`VpnService` per user/profile.

Therefore some **Kotlin/Java glue is unavoidable** — there is no pure-Rust, pure-Slint path to
capture system traffic on Android.

### 2.2 The three realistic options

**(a) Route the Go core itself as the VPN consumer via gomobile — RECOMMENDED.**
Build a `VpnService.Builder`, call `establish()`, and hand the fd straight to an embedded Go
core that owns the TUN:
- **Xray**: `AndroidLibXrayLite` (2dust) is a gomobile wrapper that produces `libv2ray.aar`.
  `CoreController.StartLoop(configContent, tunFd)` writes `xray.tun.fd` and starts Xray with the
  fd (<https://github.com/2dust/AndroidLibXrayLite/blob/master/libv2ray_main.go>). Build with
  `gomobile bind -androidapi 24 ./` (<https://github.com/2dust/AndroidLibXrayLite#readme>).
  Xray-core itself also ships Android **CLI** zips (`Xray-android-arm64-v8a.zip`, latest release
  `v26.3.27` — <https://api.github.com/repos/XTLS/Xray-core/releases/latest>), but those are
  binaries, not an AAR; the AAR comes from the wrapper.
- **sing-box**: the official `experimental/libbox` package is built into `libbox.aar` per release
  (`go run ./cmd/internal/build_libbox -target android`, merged across ABIs; CI:
  <https://github.com/SagerNet/sing-box/blob/dev/.github/workflows/build.yml>). The Kotlin side
  implements `PlatformInterface.OpenTun(options) → fd` (returns `VpnService.Builder.establish()`
  files descriptor) and libbox consumes it
  (<https://github.com/SagerNet/sing-box/blob/dev/experimental/libbox/platform.go>).
  This is exactly what sing-box-for-android does (<https://github.com/SagerNet/sing-box-for-android>).

**(b) Our own `VpnService` forwarding into a local core (desktop Proxy-mode analogue).**
Kotlin owns the TUN and a userspace forwarder; the core runs as a local SOCKS/HTTP inbound and
the forwarder dials through it. This is essentially option (c) with the core kept separate and
is what v2rayNG calls "Proxy only mode" (local `127.0.0.1:10808` — <https://github.com/2dust/v2rayNG/wiki/Mode>).
Downside: you still need a TCP/UDP forwarder (tun2socks) written by us or borrowed.

**(c) tun2socks-style.**
Replace the core's own TUN with a generic tun2socks (e.g. `hev-socks5-tunnel`, which v2rayNG
ships as a fallback for some TUN modes — <https://github.com/heiher/hev-socks5-tunnel>) reading
the fd and dialing a local SOCKS. Most moving parts, most CPU/battery; only interesting if we
want a core-agnostic forwarder.

**Decision:** take **(a)**. Reason: it reuses the cores' mature TUN/UDP implementations
(including sing-box's gVisor/system stacks and Xray's native TUN), avoids writing a userspace
forwarder, and matches both reference clients. We can still support Proxy-only later (option b)
as an opt-in low-battery mode (v2rayNG offers both).

### 2.3 Which core for Android v1

Flamix already picks a core in `flamix_core::config::plan*` (`config/mod.rs:168-300`): sing-box
whenever TUN is needed, Xray for xhttp/kcp chains. That maps cleanly:

- **sing-box `libbox`** as the primary Android core: it natively outputs Hysteria2/TUIC and owns
  the TUN; `flamix-core::config::singbox::full_config_with(..., tun=true, upstream_socks, …)`
  already generates exactly this shape.
- **Xray `libv2ray`** for xhttp-only servers: the current desktop TUN plan already chains
  Xray (local SOCKS) behind sing-box (`config/mod.rs:272-300`). On Android the same two libs
  can coexist (`libv2ray.so` + `libbox.so`); the sing-box `tun` inbound dials Xray's local
  SOCKS. This is the cleanest reuse of existing generation logic.

### 2.4 The self-traffic loop (must design for it)

The app itself fetches subscriptions and pings while the tunnel is up. Two standard mitigations:

- `VpnService.Builder.addDisallowedApplication(<self package>)` so Flamix's own UID bypasses its
  VPN. v2rayNG does this unconditionally when per-app proxy is off
  (`CoreVpnService.kt`, `configurePerAppProxy`).
- Or call `VpnService.protect(socket)` for each outbound socket
  (<https://developer.android.com/develop/connectivity/vpn>).

We need at least one of them so `flamix_core::subscription::update` and `ping.rs` keep working
while connected. Simplest: disallow self.

### 2.5 Minimal JNI surface (Rust ⇄ Kotlin)

Because Slint's Android backend is a `NativeActivity` (Rust `cdylib`), Kotlin and Rust live in
the same process. Minimal surface:

| Direction | Function | Purpose |
|---|---|---|
| Rust → Kotlin | `startVpn(configJson: String, core: Int)` (JNI call or `startService(Intent)`) | Ask `FlamixVpnService` to prepare + establish + launch the core with this config. |
| Kotlin → Rust | `nativeOnVpnEstablished(fd: int)` | Core needs the fd (or Kotlin passes fd directly to the Go AAR and only notifies Rust of state). |
| Kotlin → Rust | `nativeOnVpnRevoked()` | `onRevoke()` → controller goes to disconnected. |
| Kotlin → Rust | `nativeOnVpnError(msg: String)` | surface failures in the Slint UI. |
| Rust → Kotlin | `getVpnState()` / broadcast | optional status sync for notification/tile. |

If Kotlin talks **directly** to the Go AAR (recommended), Rust never touches the fd — it only
generates the config JSON and drives state via the Slint callback/event bridge
(`slint::android::init_with_event_listener`, <https://github.com/slint-ui/slint/blob/master/api/rs/slint/android.rs>).

---

## 3. UI: Slint on Android

### 3.1 Status: supported, with constraints

Slint supports Android in Rust and C++ via the `android-activity` crate, re-exported as
`slint::android::android_activity`. Requirements
(<https://github.com/slint-ui/slint/blob/master/docs/astro/src/content/docs/guide/platforms/mobile/android.mdx>):

- The project must be a **library** with `crate-type = ["cdylib"]`.
- Enable the feature `slint = { version = "1.x", features = ["backend-android-activity-06"] }`.
  It is "safe to enable unconditionally" (docs). Current backend deps: `android-activity 0.6.1`,
  `jni 0.22`, `ndk 0.9` (`internal/backends/android-activity/Cargo.toml`).
- Entry point is `#[unsafe(no_mangle)] fn android_main(app: slint::android::AndroidApp)`,
  then `slint::android::init(app)` and `window.run()`.
- The activity must be `android.app.NativeActivity` (or a subclass).
- **Minimum API 26** (Android 8.0) is the floor for the Slint Android backend (docs).

### 3.2 Renderer: Skia only

Slint's backend selector rejects everything but Skia on Android:
`if self.renderer.as_ref().is_some_and(|r| r != "skia") { return Err("Only the Skia renderer is supported on Android") }`
(`internal/backends/selector/api.rs:450`). FemtoVG is deliberately excluded on Android
("doesn't compile without `RUST_FONTCONFIG_DLOPEN=on` and even then can't load fonts",
`api/rs/slint/Cargo.toml:338-340`). The compile-time F-Droid recipe builds Skia from source with
the NDK (`tools/viewer/android/build-native.sh`). **Consequence:** the Flamix Android build must
enable `renderer-skia` (not `renderer-femtovg`) and the CI image needs clang/ninja/python3 for
the Skia build.

### 3.3 What of our UI works

Flamix's `.slint` uses only `std-widgets` primitives plus a custom `Theme` (checked: every
`import … "std-widgets.slint"` in `app/flamix/ui` imports `ScrollView`, `LineEdit`, `TextEdit`,
`Switch`, `ComboBox`, `SpinBox`, `Button`; no `fluent` import). So:

- **Works on Android:** all pages/components, `std-widgets`, custom theme, RU/EN strings.
- **Needs a mobile pass:** touch scrolling — `ScrollView` must set `mouse-drag-pan-enabled: true`
  for pan-to-scroll (docs); 44 dp+ touch targets and spacing (docs "Application Layout
  Considerations"); the desktop two-pane rail layout is wrong for a phone.
- **Does not work / must change:** the tray (nonexistent on Android — see §6), native file
  dialogs (`rfd`, `app/flamix/src/pages/about.rs`), clipboard via `arboard`
  (`app/flamix/src/pages/add.rs`), `--screenshot` PNG mode, `winit`.

Slint gives us the mobile plumbing for free:

- Safe area: `Window.safe-area-insets` (docs, "Safe Area"), crucial for the notch/gesture bar.
- Virtual keyboard: `Window.virtual-keyboard-position` / `virtual-keyboard-size`; Slint opens the
  IME automatically on focus and can scroll a `Flickable` to keep the field visible (docs,
  "Keyboard Handling"). This matters for the Add-server page.
- DPI/scale: handled by the backend; Skia renders at device pixel ratio (docs).
- The default Slint *style* on Android Rust is **unverified** (the C++ docs say Material is
  selected automatically; no Rust equivalent found in the clone). We use a custom `Theme`, so
  only the handful of `std-widgets` instances are affected; pin `SLINT_STYLE=material` if the
  default looks wrong.

### 3.4 Reference Android Slint apps to copy

- `tools/viewer/android/` — complete cargo-ndk + Gradle + AAB pipeline, per-ABI splits, signing,
  F-Droid recipe (`build-native.sh`, `app/build.gradle.kts`).
- `examples/todo/rust` and `examples/todo/cpp/android/` — minimal Android app.
- `examples/slide_puzzle` — touch demo.

---

## 4. Architecture proposal

### 4.1 Crate layout

Keep `flamix-core` unchanged in spirit (tiny Android fixes), replace `flamix-sys` behind a trait,
and split the Slint app into a reusable UI library + two thin platform entry points.

```
crates/
  flamix-core/            # + cfg/env fixes (§1.2); + package_name routing variant
  flamix-sys/             # desktop impls of the new platform trait (unchanged behaviour)
  flamix-platform/        # NEW (small): trait CoreHost / ProxyControl / Autostart / Stats
  flamix-android/         # NEW (Rust cdylib): android_main + JNI + Android CoreHost impl
app/
  flamix/                 # desktop bin: winit + tray, depends on flamix-ui
  flamix-ui/              # NEW: extracted lib (model.slint, view.rs, pages/*, controller core)
  android/                # NEW Gradle shell: NativeActivity, VpnService, tile, deep links, AARs
    app/src/main/java/<pkg>/MainActivity.kt          # extends NativeActivity
    app/src/main/java/<pkg>/FlamixVpnService.kt       # VpnService + core launch
    app/src/main/java/<pkg>/FlamixTileService.kt      # quick-settings tile
    app/src/main/java/<pkg>/ImportActivity.kt         # ACTION_SEND + v2rayng:// etc.
    app/src/main/jniLibs/<abi>/libflamix.so           # cargo-ndk output of flamix-android
    app/src/main/jniLibs/<abi>/libbox.so|libv2ray.so  # from libbox.aar / libv2ray.aar
    app/src/main/AndroidManifest.xml
    app/build.gradle.kts, settings.gradle.kts, gradle wrapper
```

Why a new `flamix-platform` trait instead of `cfg`-swapping inside `flamix-sys`: the controller
(`app/flamix/src/controller.rs:1`) calls `flamix_sys::{runner,elevation,stats,sysproxy,autostart}`
directly; a trait keeps desktop untouched and gives Android a compile-time alternative. The trait
methods mirror today's calls: `connect(plan) -> Handle`, `disconnect()`, `running()`,
`stats() -> (u64,u64)`, `set_proxy/unset_proxy`, `autostart`, `needs_elevation`.

### 4.2 Reuse of `controller.rs` / pages

- Extract the state machine (state mutation, `apply_import`, config generation via
  `config::plan_cluster`, ping orchestration, persist debounce) into `flamix-ui` mostly verbatim.
- The platform-specific seams are exactly: (1) core launch → `flamix-platform::CoreHost`;
  (2) system proxy → no-op on Android; (3) tray → replaced by notification/tile; (4) file
  dialogs/clipboard → Android intents/clipboard; (5) single-instance / deep links → `ImportActivity`.
- Pages/components stay shared; add responsive variants/collapsing for phones.

### 4.3 Store path mapping

On Android, app-private storage is `context.getFilesDir()` (e.g.
`/data/data/<pkg>/files`). Set `FLAMIX_HOME=<filesDir>/flamix` from Kotlin (or Rust via JNI)
**before** any `store::load`/`save`, so `store::data_dir()` (`store.rs:10`) returns it and
`state.json`/`state.json.tmp` live under app-private storage. The existing atomic
write + corrupt-file backup logic (`store.rs:52-95`) works unchanged. Subscriptions and servers
are all inside `state.json`, so no other path work is needed.

### 4.4 Flow diagram

```
                         ┌──────────────────────────────────────────┐
                         │ Slint UI (Rust cdylib, libflamix.so)      │
                         │  MainWindow / pages / controller state    │
                         └───────────────┬──────────────────────────┘
                                         │ generate config JSON
                                         v
                         ┌──────────────────────────────────────────┐
                         │ flamix-core::config (xray/singbox/routing)│
                         └───────────────┬──────────────────────────┘
                                         │ CorePlan
                                         v
     JNI / startService        ┌──────────────────────────────────────────┐
  ┌──────────────────────────► │ Kotlin FlamixVpnService (VpnService)      │
  │                            │  prepare() → Builder → establish() → fd   │
  │                            └───────┬──────────────────────┬────────────┘
  │  state events                      │ fd + config           │ protect()/disallow-self
  │  (event listener)                  v                      v
  │                            ┌──────────────────┐   ┌─────────────────────┐
  └────────────────────────────│ libbox (sing-box)│   │ flamix-core fetch/  │
                               │  tun inbound     │   │ ping (own sockets)  │
                               │  → outbound      │   └─────────────────────┘
                               └────────┬─────────┘
                                        │ if xhttp-only:
                                        v
                               ┌──────────────────┐
                               │ libv2ray (Xray)  │  local SOCKS inbound
                               └──────────────────┘
```

---

## 5. Build & CI

### 5.1 Rust native build

- Toolchain targets: `rustup target add aarch64-linux-android armv7-linux-androideabi
  x86_64-linux-android` (Slint docs).
- Build the cdylib with **cargo-ndk** (current 4.1.2, <https://crates.io/crates/cargo-ndk>):
  `cargo ndk -t arm64-v8a -t armeabi-v7a -t x86_64 -o app/android/app/src/main/jniLibs
  build --release -p flamix-android`.
- NDK: Slint's own Android CI uses NDK **r27** (`tools/viewer/android/build-native.sh`) and r28
  for sing-box (<https://github.com/SagerNet/sing-box/blob/dev/.github/workflows/build.yml>).
  Pin one NDK (r27/r28) in CI.
- Set `ANDROID_JAR` / `ANDROID_BUILD_TOOLS_VERSION` because the android-activity backend compiles
  and embeds a Java helper (`build-native.sh`); set `ANDROID_NDK` so Skia builds from source.

### 5.2 Gradle packaging

- Gradle wrapper + Android Gradle Plugin 8.x; `compileSdk` 35/36, `minSdk = 26` (Slint floor),
  `targetSdk` current.
- `abiFilters` per ABI; per-ABI APK splits (v2rayNG) or a single AAB for Play (sing-box/Slint).
- Signing: `-Pandroid.injected.signing.store.file=…` (v2rayNG) or a `signingConfigs` block from
  env vars (Slint viewer). Store the keystore in CI secrets.
- **APK vs AAB:** AAB for Play (required), APK for GitHub/F-Droid. Both from the same Gradle
  project.
- Core binaries: reuse **upstream Android builds**. Because Play forbids downloading executable
  code (`.so`/dex) from outside Play
  (<https://support.google.com/googleplay/android-developer/answer/9888379>), the core `.so` must
  be **bundled in the APK/AAB** (or shipped as a Play asset/feature module). Pin a specific
  `libbox.aar` (sing-box release) and `libv2ray.aar` (AndroidLibXrayLite release) by tag+hash.
  Building Xray/sing-box from source is only needed if we need a custom build tag set; sing-box
  publishes `DEFAULT_BUILD_TAGS` for packagers
  (<https://github.com/SagerNet/sing-box/blob/dev/docs/installation/build-from-source.md>).

### 5.3 GitHub Actions sketch

```yaml
android:
  runs-on: ubuntu-latest
  steps:
    - uses: actions/checkout@v4
    - uses: actions/setup-java@v4   # temurin 21
    - uses: nttld/setup-ndk@v1      # r28 (matches sing-box)
    - uses: dtolnay/rust-toolchain@stable
      with: { targets: aarch64-linux-android, armv7-linux-androideabi, x86_64-linux-android }
    - run: cargo install cargo-ndk
    - run: |
        # fetch pinned AARs (libbox.aar, libv2ray.aar) from upstream releases by tag+sha256
        cargo ndk -t arm64-v8a -t armeabi-v7a -t x86_64 \
          -o app/android/app/src/main/jniLibs build --release -p flamix-android
    - run: cd app/android && ./gradlew assembleRelease bundleRelease
    - uses: actions/upload-artifact@v4
      with: { path: app/android/app/build/outputs/** }
```

The existing desktop workflows (`.github/workflows/{ci,packages,release}.yml`) build
Windows/macOS/Linux; Android should be a **separate job** (Android SDK/NDK + Java + Skia are
heavy and orthogonal to the Rust-only CI in `ci.yml`).

---

## 6. Feature deltas vs desktop (honest)

| Area | Desktop today | Android requirement |
|---|---|---|
| Traffic capture | Proxy mode (system proxy + local SOCKS/HTTP) or TUN via sing-box elevated (`flamix-sys::runner`) | **VPN permission flow** (`VpnService.prepare()` → user dialog) is mandatory; no system-proxy mode on stock Android (only Wi-Fi HTTP proxy, manual). |
| Process lifetime | Parent process owns child cores | **Foreground service + persistent, non-dismissible notification** (API 26+); for API 34+ declare `foregroundServiceType` + the `FOREGROUND_SERVICE_SPECIAL_USE` permission and a Play FGS declaration. v2rayNG uses `foregroundServiceType="specialUse"`, `BIND_VPN_SERVICE`, and `PROPERTY_SPECIAL_USE_FGS_SUBTYPE=vpn`. |
| Per-app proxy | n/a (`gaps.md` #24) | **The one mobile killer feature**: `Builder.addAllowedApplication`/`addDisallowedApplication`; cannot mix allowed and disallowed; must set before `establish()` (<https://developer.android.com/develop/connectivity/vpn>). Requires a package picker UI (`QUERY_ALL_PACKAGES`), and routing by `package_name` not `process_name`. |
| Always-on / reboot | autostart via desktop service | `android:name="android.net.VpnService.SUPPORTS_ALWAYS_ON"` + `BOOT_COMPLETED`/`MY_PACKAGE_REPLACED` receiver (v2rayNG manifest). |
| Network handover | n/a | `ConnectivityManager` default-network monitor + `setUnderlyingNetworks()` and/or restart the core, to survive Wi-Fi↔cellular; v2rayNG reloads the core in place (`CoreServiceManager.NetworkMonitor`). |
| Battery/doze | n/a | Foreground service; avoid polling when idle; expected higher drain in TUN mode (v2rayNG wiki notes Proxy-only is lighter). |
| Quick Settings / shortcuts | tray menu | **Quick-settings tile** (`TileService`, `BIND_QUICK_SETTINGS_TILE`), app shortcuts, home-screen widget (v2rayNG). Replaces the tray entirely — **there is no tray on Android**. |
| Import | file dialogs + clipboard (`rfd`/`arboard`) + single-instance args | `ACTION_SEND` (`text/plain` share), `ACTION_VIEW` deep links: `flamix://`, `v2rayng://install-sub|install-config?url=…#name`, `clash://`, `sing-box://`, `hiddify://` (`docs/research/v2rayng.md`). Clipboard via Android `ClipboardManager`. |
| QR import/export | none | Camera scan / QR display (v2rayNG uses CameraX + zxing). Nice-to-have. |
| Metrics | Xray CLI + sing-box clash API (`flamix-sys::stats`) | Use the **clash API** (`singbox_traffic`) or the Go AAR's stats call; there is no `xray` CLI. |
| Tray | ✅ | ❌ (no Android concept). |

**Out of scope for v1:** per-app proxy UI (design the plumbing but ship later), quick-settings
tile, widgets, Tasker/Locale integration, QR scan, root/tproxy and LAN sharing, always-on
polish, Play Store submission, iOS.

---

## 7. Risks & unknowns (ranked)

1. **Slint-on-Android maturity for a full app.** Impact: high (whole UI). The backend is Skia-only,
   requires `NativeActivity`, and our desktop feature set (`winit`, tray, `rfd`) does not exist
   there. De-risk cheaply: build the Slint `examples/todo/rust` Android example and the
   `tools/viewer/android` app, run them on a device; port one Flamix page and test IME + safe
   areas before committing to the layout.
2. **Two-native-runtime coordination (Rust cdylib + Go AAR).** Impact: high (the core of the
   port). fd passing, `protect()`/self-exclusion, process death vs service lifecycle, and
   keeping Rust's config generation in sync with a config the Go core accepts. De-risk: a
   throwaway slice that calls `libbox.OpenTun`/`CoreController.StartLoop` with the fd and prints
   egress IP.
3. **`reqwest`/`rustls` build + runtime under VPN.** Impact: medium-high. `rustls` provider on
   `aarch64-linux-android` is **unverified**; and without self-exclusion the app's own DNS/TLS
   loops through its own tunnel. De-risk: `cargo ndk build -p flamix-core` early; test a
   subscription fetch while connected with `addDisallowedApplication(self)`.
4. **Play Store distribution for a VPN app.** Impact: high if Play is a target. FGS types must be
   declared and justified (Android 14+); proxy apps are only allowed if proxying is the primary
   user-facing purpose; no executable code may be downloaded outside Play (so cores must be
   bundled). De-risk: ship GitHub/F-Droid first; treat Play as a later compliance exercise.
5. **Core build/packaging and version pinning.** Impact: medium. `libbox.aar`/`libv2ray.aar`
   are upstream artifacts (gomobile/NDK); a bad pin breaks the build or runtime. De-risk: record
   the exact tag + sha256 in-repo and add a CI integrity check.
6. **Store path / permissions / `directories` on Android.** Impact: medium. De-risk: force
   `FLAMIX_HOME` from `filesDir`; add a unit test around `store::load/save` with a temp dir.
7. **Refactor coupling (`flamix-sys` → trait, UI extraction).** Impact: medium. A wrong seam
   touches the desktop app. De-risk: introduce the trait behind a default desktop impl and keep
   `scripts/ru1 cargo test` green before adding Android.
8. **Android routing semantics.** Impact: medium. `process_name` rules don't work; DNS/`strict_route`
   differences (sing-box docs: `strict_route`, `include_interface`, `exclude_uid` unavailable).
   De-risk: generate `package_name`-based rules for Android; validate against sing-box docs.

---

## 8. Suggested task breakdown

Sizes: S ≤ ~1 day, M ≈ 2–4 days, L ≈ 1–2 weeks (rough). Order = dependency order.

| # | Task | Size | Depends on |
|---|---|---|---|
| A1 | Introduce `flamix-platform` trait (CoreHost/Proxy/Autostart/Stats) + desktop impl; make `controller.rs` depend on it. **No behaviour change.** | S | — |
| A2 | `flamix-core` Android audit: document/gate `data_dir` (env `FLAMIX_HOME`), add `package_name` TUN routing variant to `config/singbox.rs`, tests. | S | — |
| A3 | Extract `app/flamix-ui` lib (model.slint, view, pages, controller core) from `app/flamix`; desktop bin keeps winit+tray. | M | A1 |
| B1 | `crates/flamix-android` cdylib skeleton: `android_main`, `slint::android::init`, reuse `flamix-ui`, `FLAMIX_HOME` from files dir, a "Connect" button that just toggles UI state. | M | A2, A3 |
| B2 | Gradle shell `app/android`: `NativeActivity`, manifest/permissions, debug apk via cargo-ndk; CI job. | M | B1 |
| C1 | Kotlin `FlamixVpnService`: `prepare()` flow, `Builder` (address/route/DNS/MTU), `establish()`, foreground notification, `onRevoke()`, self-exclusion. No core yet (fd logged). | M | B2 |
| C2 | JNI/state bridge Rust⇄Kotlin (event listener + `startVpn`/`onRevoked`). | S | C1 |
| D1 | Embed **sing-box `libbox.aar`**: Kotlin `PlatformInterface.OpenTun` returns the fd; push `flamix-core`-generated TUN config; verify egress. | L | C2 |
| D2 | Embed **Xray `libv2ray.aar`** for xhttp-only servers; chain behind sing-box (mirror desktop TUN chain). | M | D1 |
| E1 | Mobile UX pass: responsive pages, `mouse-drag-pan-enabled`, safe-area insets, IME handling, clipboard. | M | B1 |
| E2 | Imports: `ImportActivity` for `ACTION_SEND` + `flamix://`/`v2rayng://`/`clash://`/`sing-box://`. | S | C1 |
| E3 | Per-app proxy: package picker, `addAllowed/DisallowedApplication`, `package_name` routing. | M | D1, A2 |
| E4 | Quick-settings tile + notification actions + network-handover reload. | M | C1 |
| F1 | Packaging/CI: AAB + per-ABI APKs, signing, pinned AARs with sha256, release job. | M | B2, D1 |
| F2 | Docs: `docs/build.md` Android section, README platform row. | S | F1 |

### First vertical slice (proves the risky part early)

**"Connect → TUN → one config → real egress."** Build the smallest possible APK that:

1. Shows a single Slint screen (reusing `flamix-ui`) with a Connect toggle.
2. On tap, runs the `VpnService.prepare()` permission flow and starts `FlamixVpnService`.
3. The service establishes a TUN and hands the fd + a **VLESS config generated by
   `flamix_core::config`** to an embedded sing-box `libbox`.
4. Proves a real HTTPS request (browser) exits through the tunnel **and** Flamix's own
   subscription fetch still works (self-exclusion).

This exercises every risky integration (Slint Android backend, Kotlin `VpnService`, fd passing
to the Go core, `flamix-core` config reuse, self-traffic loop) in one runnable artifact.
Tasks: **B1 + B2 + C1 + C2 + D1** (plus A1–A3 as prerequisites). Nothing else should start
before this slice succeeds.

---

## Sources

Local (cloned for this task into `/srv/dev/tmp/flamix-android-research/`, `--depth 1`):
- Slint `slint-ui/slint` (docs `guide/platforms/mobile/{android,general}.mdx`,
  `api/rs/slint/{Cargo.toml,android.rs}`, `internal/backends/{selector/api.rs,android-activity/*}`,
  `tools/viewer/android/*`, `examples/*`).
- v2rayNG `2dust/v2rayNG` (`AndroidManifest.xml`, `CoreVpnService.kt`, `CoreServiceManager.kt`,
  `CoreNativeManager.kt`, `app/build.gradle.kts`, `.github/workflows/build.yml`, `.gitmodules`,
  wiki "Mode").
- AndroidLibXrayLite `2dust/AndroidLibXrayLite` (`libv2ray_main.go`, `README.md`).
- sing-box `SagerNet/sing-box` (`experimental/libbox/{platform,tun,service}.go`,
  `docs/clients/android/*`, `docs/installation/build-from-source.md`,
  `.github/workflows/build.yml`, `Makefile`).

External URLs (fetched read-only):
- Slint Android guide: <https://slint.dev> docs mirrored at
  <https://github.com/slint-ui/slint/blob/master/docs/astro/src/content/docs/guide/platforms/mobile/android.mdx>
- Slint mobile general guide (safe area / keyboard):
  <https://github.com/slint-ui/slint/blob/master/docs/astro/src/content/docs/guide/platforms/mobile/general.mdx>
- Android `VpnService` guide: <https://developer.android.com/develop/connectivity/vpn>
- `android-activity` crate: <https://crates.io/crates/android-activity> (0.6.1)
- `cargo-ndk` crate: <https://crates.io/crates/cargo-ndk> (4.1.2)
- Xray-core latest release (Android CLI zips): <https://api.github.com/repos/XTLS/Xray-core/releases/latest>
- AndroidLibXrayLite: <https://github.com/2dust/AndroidLibXrayLite>
- sing-box for Android: <https://github.com/SagerNet/sing-box-for-android>
- sing-box Android feature limits: <https://github.com/SagerNet/sing-box/blob/dev/docs/clients/android/features.md>
- sing-box build/packaging: <https://github.com/SagerNet/sing-box/blob/dev/docs/installation/build-from-source.md>
- Google Play, Device and Network Abuse (proxy primary purpose, no outside-Play code):
  <https://support.google.com/googleplay/android-developer/answer/9888379>
- Google Play, Deceptive Behavior: <https://support.google.com/googleplay/android-developer/answer/9888077>
- hev-socks5-tunnel (tun2socks option): <https://github.com/heiher/hev-socks5-tunnel>

Marked **unverified**: `directories::ProjectDirs` on Android; `rustls` provider build on
`aarch64-linux-android`; Slint's default style on Android from Rust; exact `libbox` gomobile
entry points beyond `OpenTun`; Android/Build target-SDK specifics for the Play FGS declaration.

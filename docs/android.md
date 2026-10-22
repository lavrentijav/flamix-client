# Android: architecture and plan

Status: **M1 done, M2 in progress** (see Milestones). Android comes first; iOS follows (needs an Apple Developer account and a tester device).

## What runs where

- **UI**: the same Slint app, built for `aarch64-linux-android` (and `x86_64` for emulators). A phone layout
  replaces the desktop three-panel window: bottom tabs (Серверы, Добавить, Маршрутизация, Настройки, Статистика),
  the connection button on the servers tab, swipe-free, touch targets at least 48 dp.
- **Tunnel**: Android allows VPN only through `VpnService`, which the user must allow once. The app
  establishes a TUN interface in Kotlin, gets its file descriptor and hands it to the core.
- **Core**: the proxy core (sing-box, the same family we use on desktop) runs as a native executable shipped
  inside the APK as `lib<name>.so` (`jniLibs`, `extractNativeLibs=true`). Android 10+ executes binaries from
  `nativeLibraryDir`; this is how the core is started. Pure-Go cores cross-compile for Android with
  `GOOS=android GOARCH=arm64 CGO_ENABLED=0`, so no NDK toolchain is needed for the core itself.
- **Sing-box on Android**: its `tun` inbound accepts an existing file descriptor (`file_descriptor`), so the
  VpnService TUN is used directly and no separate tun2socks is needed. The fd is passed to the child process
  by inheriting it across fork/exec (the core is started from Rust with the fd kept open).
- **Proxy-only mode** (no VPN permission): a local mixed inbound, same as desktop Proxy mode. Useful for
  Wi-Fi/system-proxy setups and for testing.

## Layout (to be created)

```
android/                  Gradle project (Kotlin shell)
  app/src/main/
    AndroidManifest.xml   VpnService, foreground service, notification permission
    java/online/flamix/   MainActivity (hosts Slint), FlamixVpnService, JNI bridge
    jniLibs/arm64-v8a/    libsingbox.so, libxray.so, libmihomo.so (built in CI)
crates/flamix-sys/src/android.rs   start/stop core with the VPN fd, log tail
app/flamix/src/android_main.rs     android-activity entry point (slint android backend)
```

## Build

- CI job `android.yml` on `ubuntu-latest`: Rust target `aarch64-linux-android`, Android SDK and NDK (preinstalled
  on the runner or installed by `android-actions/setup-android`), Go for the cores, then Gradle assembles the APK.
- Signing: a debug key for CI artifacts; a release keystore kept as a repository secret when we publish.
- Local builds on ru1 are possible but slow; CI is the default.

## Milestones

1. **M1 — toolchain**: an empty Slint window runs in an APK on an emulator (CI builds it, artifact uploaded).
2. **M2 — core**: sing-box for Android cross-built in CI; Proxy-only mode works on the emulator (curl through `127.0.0.1:10808`).
3. **M3 — VPN**: `VpnService` + fd handoff; TUN mode works on a real phone (needs the user's device).
4. **M4 — UI**: phone layout for all tabs; reuse the desktop components where they fit.
5. **M5 — release**: signed APK/AAB attached to the GitHub release.

## Open questions

- Play Store distribution is not planned for now; APK from GitHub Releases only.
- Which Android versions to support: minimum API 26 (Android 8) proposed.

## M2: core in the app (proxy-only)

- CI job `singbox` downloads the official prebuilt sing-box (`SINGBOX_TAG` in `.github/workflows/android.yml`,
  android arm64 -> `arm64-v8a`, amd64 -> `x86_64`), checks the pinned SHA256 and installs it as `libsingbox.so`; the `apk` job drops it into `jniLibs/`.
- `CoreService.kt` runs it from `nativeLibraryDir` with the sample config `assets/singbox-sample.json`
  (mixed inbound `127.0.0.1:10808`, direct outbound), copied to `filesDir` on start.
- Debug trigger (debuggable builds only):
  `adb shell am start -n online.flamix.client/online.flamix.MainActivity --es core start` (or `--es core stop`).
  Log tag `FlamixCore`.
- `scripts/android-m2-check.sh <apk>` does this on an emulator; CI job `emulator` runs it (optional, `continue-on-error`).

## M4: phone screen (first slice)

- Window: `app/flamix-android/ui/android.slint` (`AndroidWindow`), built by `build.rs`. Top bar "Flamix", round connect
  button (power icon), selected server, server list (tap to select), bottom tabs Серверы / Добавить / Настройки.
  Settings is a placeholder.
- State: `<files dir>/state.json` via `flamix_core::store::load/save`, files dir from `AndroidApp::internal_data_path()`.
  Logic and unit tests: `app/flamix-android/src/phone.rs`.
- Add: `subscription::update` on a plain thread; the result is applied on the UI thread through `invoke_from_event_loop`.
  The clipboard button is not done yet.
- Connect: `phone::vpn_config_json` -> `android_tun::android_config` -> `bridge.rs` JNI call to
  `online.flamix.FlamixBridge.connect` (Kotlin) -> `MainActivity.startVpn` (consent) -> `FlamixVpn.start`.
  The button state is polled from `FlamixBridge.isRunning` once per second.

## Release

- Signing: `android/app/build.gradle.kts` signs `release` only when `ANDROID_KEYSTORE_PATH` is set
  (PKCS12; also `ANDROID_KEYSTORE_PASSWORD`, `ANDROID_KEY_ALIAS`, `ANDROID_KEY_PASSWORD`). Without it the release APK is
  unsigned and debug builds are unchanged.
- **The signing key must never change.** Android only installs an update if it is signed with the same key as the
  installed app; a new key means users must uninstall first. Keep a backup of the keystore outside GitHub.
- Secrets (repository secrets): `ANDROID_KEYSTORE_BASE64` (base64 of the PKCS12, alias `flamix`),
  `ANDROID_KEYSTORE_PASSWORD`, `ANDROID_KEY_PASSWORD`, `ANDROID_KEY_ALIAS`. The `release` job decodes the keystore into
  `$RUNNER_TEMP`, deletes it at the end, and never commits it.
- Cut a release: `git tag vX.Y.Z && git push origin vX.Y.Z`. CI builds `Flamix-X.Y.Z-arm64-v8a.apk` and
  `Flamix-X.Y.Z-x86_64.apk`, verifies them with `apksigner`, and attaches them with `SHA256SUMS.txt` to the GitHub release.
  versionName = tag, versionCode = major*1000000 + minor*1000 + patch. `workflow_dispatch` builds the APKs as an artifact only.
- Bumping sing-box: change `SINGBOX_TAG`, `SINGBOX_VERSION` and both `SINGBOX_SHA256_*` (digests from the release assets API).

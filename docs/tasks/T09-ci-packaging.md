# T09 — CI (GitHub Actions) + packaging for Windows, macOS, Linux

Read `AGENTS.md` first. The repo is public on GitHub (`lavrentijav/flamix-client`), so Actions minutes are free.
Development builds run on ru1 (`scripts/ru1`); **release artifacts are built by GitHub Actions**.
You cannot push or watch Actions yourself — write the workflows so they are correct on the first run, and
validate every script you can locally/on ru1. The team lead pushes and reports failures back to you.

## Setup
    git -C /srv/dev/projects/flamix-client worktree add /srv/dev/projects/flamix-client-wt/T09 -b oc/T09
    cd /srv/dev/projects/flamix-client-wt/T09

## Files you own
`.github/**`, `packaging/**`, `scripts/fetch-cores.sh`, `scripts/fetch-cores.ps1`, `scripts/package-*.sh`, `README.md`
(sections "Download / Install" and "Build" only), `app/flamix/Cargo.toml` `[package.metadata.*]` sections only.

## Cores
`scripts/fetch-cores.sh <os> <arch> <out-dir>` (+ `.ps1` twin for Windows runners, or use bash on Windows runners —
`shell: bash` works there): downloads pinned versions (variables at the top; use the latest stable releases, look them
up via `https://api.github.com/repos/XTLS/Xray-core/releases/latest` and `.../SagerNet/sing-box/releases/latest` and pin
the result) of **Xray-core** (`Xray-<os>-<arch>.zip`, also brings `geoip.dat`, `geosite.dat`) and **sing-box**
(`sing-box-<ver>-<os>-<arch>.tar.gz` / `.zip`), verifies SHA256 when the release publishes digests (Xray ships `.dgst` files),
extracts `xray[.exe]`, `sing-box[.exe]`, `geoip.dat`, `geosite.dat` into `<out-dir>`. Also fetch
`geoip.dat`/`geosite.dat` from `Loyalsoldier/v2ray-rules-dat` latest (better RU lists) overriding Xray's ones.
Test it on ru1 for linux amd64: `scripts/ru1 scripts/fetch-cores.sh linux amd64 cores` and check the binaries run
(`cores/xray version`, `cores/sing-box version`).

## Workflows
1. `.github/workflows/ci.yml` — on push/PR to main: ubuntu: `cargo fmt --check`, `cargo clippy --workspace --all-targets -D warnings`,
   `cargo test --workspace`; plus `cargo check -p flamix` on windows-latest and macos-latest (catch platform code breakage).
   Cache with `Swatinem/rust-cache`. Linux system deps for Slint/winit: `libxkbcommon-dev libfontconfig1-dev libwayland-dev
   libx11-dev libxcursor-dev libxrandr-dev libxi-dev libgl1-mesa-dev` (check what the build actually needs).
2. `.github/workflows/release.yml` — on tag `v*` and `workflow_dispatch`: matrix
   - Linux x86_64 (`ubuntu-22.04` for an old glibc) and aarch64 (`ubuntu-22.04-arm`): binary + cores → `.deb` (cargo-deb),
     `.rpm` (cargo-generate-rpm), `.AppImage` (appimagetool / linuxdeploy), `.tar.gz`; generate Arch `PKGBUILD` from
     `packaging/arch/PKGBUILD.in` with the tarball sha256 (attach it as a release asset).
     Install paths: `/usr/bin/flamix`, cores in `/usr/lib/flamix/cores/`, `.desktop` in `/usr/share/applications`,
     icons in hicolor (from `app/flamix/assets/app-icon/` PNGs or the SVG). `.desktop` registers `x-scheme-handler/flamix`
     (MimeType) and `Exec=flamix %u`. deb depends: `libxkbcommon0, libfontconfig1, libgl1 | libegl1`; rpm equivalent;
     postinst: `setcap cap_net_admin,cap_net_bind_service+ep /usr/lib/flamix/cores/sing-box || true` (TUN without pkexec).
   - Windows x86_64 (`windows-latest`, MSVC): `flamix.exe` + cores → portable `.zip` and an installer: Inno Setup
     (`iscc` is preinstalled on windows runners) script `packaging/windows/flamix.iss`: per-user install by default,
     Start menu + optional desktop shortcut, registers `flamix://` URL protocol in HKCU, uninstaller, app icon (`.ico`
     generated from the PNGs in CI with ImageMagick `magick` — preinstalled). Embed the icon + a manifest into the exe
     via `build.rs`-free approach: `winresource` build-dependency in `app/flamix` guarded by `cfg(windows)` is allowed
     (you may edit `app/flamix/build.rs` only to add that, keep the slint part).
   - macOS universal (`macos-latest`): build `aarch64-apple-darwin` + `x86_64-apple-darwin`, `lipo` into universal, cores for both
     archs lipo'd too (or arm64+x86_64 separately if lipo fails), `.app` bundle (`packaging/macos/Info.plist` with
     `CFBundleURLTypes` for `flamix`, `LSMinimumSystemVersion 11.0`, `.icns` from PNGs via `iconutil`), cores in
     `Flamix.app/Contents/Resources/cores/`, ad-hoc sign (`codesign --force --deep -s -`), `.dmg` via `hdiutil` with an
     `/Applications` symlink.
   - Final job: create/update the GitHub Release for the tag (`softprops/action-gh-release`) with all artifacts and
     `SHA256SUMS.txt`. For `workflow_dispatch` just upload artifacts (no release).
3. Pin action versions to major tags (`actions/checkout@v4`, ...). Least-privilege `permissions:` (contents: write only on the release job).

## Local check on ru1 (required)
`scripts/package-linux.sh` that does the Linux packaging steps (used by CI too, so CI and ru1 behave the same):
build release, fetch cores, make `dist/flamix_<ver>_amd64.deb`, `.rpm` (if `rpmbuild`/cargo-generate-rpm available — cargo-generate-rpm
needs no rpmbuild), `.tar.gz`, `PKGBUILD`, AppImage (download appimagetool to `~/flamix-tools/` if missing; it needs FUSE to *run*,
but `--appimage-extract-and-run` works without it). Run it: `FLAMIX_FETCH=dist scripts/ru1 scripts/package-linux.sh` and list the
produced files with sizes in your report. `dist/` is gitignored. Use `cargo install --locked cargo-deb cargo-generate-rpm` on ru1 into
`~/.cargo/bin` (allowed, user-level).
Validate YAML: `python3 -c "import yaml,sys; [yaml.safe_load(open(f)) for f in sys.argv[1:]]" .github/workflows/*.yml`, and if
`actionlint` can be downloaded as a single binary to `~/flamix-tools/bin` on ru1, run it too.

## README
"Download / Install" (per OS, incl. Arch via PKGBUILD, Fedora/RHEL rpm, Debian/Ubuntu deb, AppImage, macOS Gatekeeper
note for the ad-hoc signature: `xattr -dr com.apple.quarantine /Applications/Flamix.app`) and "Build" (`scripts/ru1`, CI).

## Done
Commit on `oc/T09`. Report: branch, commit, produced packages on ru1, pinned core versions, anything you could not verify.

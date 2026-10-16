# T16 — fix the Windows build + catch Windows breakage before CI

Read `AGENTS.md` first (build rule: `scripts/ru1`; work incrementally).

## Setup
    git -C /srv/dev/projects/flamix-client worktree add /srv/dev/projects/flamix-client-wt/T16 -b oc/T16
    cd /srv/dev/projects/flamix-client-wt/T16

## The failure (GitHub Actions, windows-latest, `cargo check -p flamix`)
    error[E0432]: unresolved import `windows_sys::Win32::UI::Shell::ShellExecuteW`
    error: could not compile `flamix-sys` (lib)
Linux and macOS jobs are green.

## Files you own
`crates/flamix-sys/**` (Windows cfg code and its Cargo.toml features), `.github/workflows/ci.yml`,
new `scripts/check-windows.sh`, `docs/build.md` (a short "Cross-checking Windows from Linux" section).

## Do
1. Make a Windows check reproducible on the build server (no root there):
   - download a pinned **zig** release tarball (https://ziglang.org/download/) into `~/flamix-tools/zig/` on the build
     server and `cargo install --locked cargo-zigbuild` (user-level, into ~/.cargo/bin);
   - `rustup target add x86_64-pc-windows-gnu`;
   - write `scripts/check-windows.sh`: `cargo zigbuild --target x86_64-pc-windows-gnu -p flamix` (falls back to
     `cargo check --target x86_64-pc-windows-gnu` with `CC_x86_64_pc_windows_gnu="zig cc -target x86_64-windows-gnu"` if zigbuild
     is unavailable) plus `cargo clippy --target x86_64-pc-windows-gnu --workspace -- -D warnings`. Run it via
     `scripts/ru1 scripts/check-windows.sh`. Zig is only a C toolchain/linker for crates like `ring`; no Zig code in the repo.
   - Reproduce the E0432 locally there first.
2. Fix it: enable the right `windows-sys` features (in 0.59 `ShellExecuteW` lives in `Win32::UI::Shell` and its signature
   pulls in `Win32_UI_WindowsAndMessaging` for `SW_*`/`HWND` types — check docs.rs for windows-sys 0.59 and enable exactly what's
   needed). Fix every further Windows error/warning that appears (the whole workspace incl. `app/flamix` must build for
   `x86_64-pc-windows-gnu`; `-D warnings` clean, `unsafe` only with `// SAFETY:` comments).
3. Also try `aarch64-apple-darwin` with `cargo check` + zig (`CC_aarch64_apple_darwin="zig cc -target aarch64-macos"`) for
   `flamix-sys` only; if the macOS SDK is required for the app, skip the app and say so — CI covers macOS.
4. CI (`ci.yml`): Windows and macOS jobs run `cargo clippy --workspace --all-targets -- -D warnings` and
   `cargo test -p flamix-core -p flamix-sys` instead of only `cargo check -p flamix`; bump `actions/checkout` to v5
   (Node 20 deprecation warning). Keep the Linux job as is.

## Done
    scripts/ru1 scripts/check-windows.sh          (green)
    scripts/ru1 cargo clippy --workspace --all-targets -- -D warnings
    scripts/ru1 cargo test -p flamix-sys
Commit on `oc/T16`. Report: branch, commit, root cause, zig/cargo-zigbuild versions, what CI now runs.

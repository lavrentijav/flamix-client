#!/usr/bin/env bash
# Build Linux release packages for the current architecture:
#   dist/flamix_<ver>_<debarch>.deb        (cargo-deb, optional)
#   dist/flamix-<ver>-1.<rpmarch>.rpm      (cargo-generate-rpm, optional)
#   dist/flamix-<ver>-linux-<arch>.tar.gz
#   dist/flamix-<ver>-linux-<arch>.AppImage
#   dist/PKGBUILD                          (x86_64 only, from packaging/arch/PKGBUILD.in)
#
# Runs on any Linux machine (including CI); it only needs a Rust toolchain and
# the Slint system dependencies — see docs/build.md. `.deb`/`.rpm` additionally
# need cargo-deb / cargo-generate-rpm, the AppImage needs curl:
#
#   cargo install --locked cargo-deb cargo-generate-rpm
#   scripts/package-linux.sh
#
# Environment overrides:
#   FLAMIX_CORES_DIR    use pre-downloaded cores instead of running fetch-cores.sh
#   FLAMIX_DOWNLOAD_URL URL of the tar.gz referenced by the generated PKGBUILD
#   FLAMIX_TOOLS        directory to cache appimagetool (default: ~/.cache/flamix)
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

REPO_URL="https://github.com/lavrentijav/flamix-client"
VER="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n1)"
[ -n "$VER" ] || { echo "error: cannot read workspace version from Cargo.toml" >&2; exit 1; }

case "$(uname -m)" in
  x86_64 | amd64)
    RPM_ARCH=x86_64; TARBALL_ARCH=x86_64; CORE_ARCH=amd64; AIT_ARCH=x86_64 ;;
  aarch64 | arm64)
    RPM_ARCH=aarch64; TARBALL_ARCH=aarch64; CORE_ARCH=arm64; AIT_ARCH=aarch64 ;;
  *)
    echo "error: unsupported architecture '$(uname -m)'" >&2; exit 1 ;;
esac

TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/target}"
DIST="$ROOT/dist"
STAGE="$DIST/linux-staging"
rm -rf "$STAGE"
mkdir -p "$STAGE/cores" "$STAGE/icons"

# Slint links libfontconfig on Linux. Prefer the system dev package (see
# docs/build.md); on Debian/Ubuntu without sudo we can stage it into a private
# prefix as a best-effort fallback. Anything else is a clear actionable error.
setup_fontconfig() {
    if pkg-config --exists fontconfig 2>/dev/null; then
        return 0
    fi
    if ! command -v apt-get >/dev/null 2>&1 || ! command -v dpkg-deb >/dev/null 2>&1; then
        echo "error: fontconfig development files not found." >&2
        echo "Install the Slint system dependencies first (see docs/build.md), e.g." >&2
        echo "  sudo apt install libfontconfig1-dev pkg-config" >&2
        exit 1
    fi
    echo "--- fontconfig dev files missing; staging libfontconfig-dev into a private prefix ---"
    local root="${FLAMIX_SYSROOT:-$HOME/.cache/flamix-sysroot}"
    local prefix="$root/usr"
    mkdir -p "$root/debs" "$prefix" "$root/pc"
    ( cd "$root/debs" && apt-get download libfontconfig-dev libfontconfig1 >/dev/null 2>&1 ) || true
    local deb
    for deb in "$root/debs/"*.deb; do
        [ -f "$deb" ] && dpkg-deb -x "$deb" "$root"
    done
    # keep the linker on the shared library (the -dev package also ships a .a)
    rm -f "$prefix/lib/"*/libfontconfig.a
    local src
    src="$(find "$root" -path '*/pkgconfig/fontconfig.pc' | head -n1)"
    [ -n "$src" ] || { echo "error: fontconfig.pc not found in libfontconfig-dev" >&2; exit 1; }
    sed -e "s|^prefix=.*|prefix=$prefix|" -e '/^Requires/d' "$src" > "$root/pc/fontconfig.pc"
    export PKG_CONFIG_PATH="$root/pc${PKG_CONFIG_PATH:+:$PKG_CONFIG_PATH}"
}

echo "=== flamix $VER — linux/$TARBALL_ARCH ==="

setup_fontconfig

echo "--- build release binary ---"
cargo build --release -p flamix
BIN="$TARGET_DIR/release/flamix"
[ -x "$BIN" ] || { echo "error: binary not found at $BIN" >&2; exit 1; }

echo "--- fetch cores ---"
if [ -n "${FLAMIX_CORES_DIR:-}" ]; then
  echo "using pre-fetched cores from $FLAMIX_CORES_DIR"
  cp -f "${FLAMIX_CORES_DIR%/}/"* "$STAGE/cores/"
else
  scripts/fetch-cores.sh linux "$CORE_ARCH" "$STAGE/cores"
fi

echo "--- icons ---"
bash packaging/make-icons.sh "$STAGE/icons" 16 32 48 64 128 256 512

echo "--- stage desktop file ---"
cp packaging/linux/flamix.desktop "$STAGE/flamix.desktop"

# --- .tar.gz (also the source for the Arch PKGBUILD) -------------------------
BUNDLE="flamix-${VER}-linux-${TARBALL_ARCH}"
BUNDLE_DIR="$DIST/$BUNDLE"
rm -rf "$BUNDLE_DIR"
mkdir -p "$BUNDLE_DIR"
cp "$BIN" "$BUNDLE_DIR/flamix"
cp -r "$STAGE/cores" "$BUNDLE_DIR/cores"
cp "$STAGE/flamix.desktop" "$BUNDLE_DIR/flamix.desktop"
cp -r "$STAGE/icons" "$BUNDLE_DIR/icons"

echo "--- tar.gz ---"
tar -C "$DIST" -czf "$DIST/${BUNDLE}.tar.gz" "$BUNDLE"

# --- PKGBUILD (x86_64 only) --------------------------------------------------
if [ "$TARBALL_ARCH" = "x86_64" ]; then
  echo "--- PKGBUILD ---"
  SHA="$(sha256sum "$DIST/${BUNDLE}.tar.gz" | awk '{print $1}')"
  URL="${FLAMIX_DOWNLOAD_URL:-$REPO_URL/releases/download/v${VER}/${BUNDLE}.tar.gz}"
  sed -e "s|@@VERSION@@|${VER}|g" \
      -e "s|@@URL@@|${URL}|g" \
      -e "s|@@SHA256@@|${SHA}|g" \
      packaging/arch/PKGBUILD.in > "$DIST/PKGBUILD"
fi

# --- .deb (cargo-deb) --------------------------------------------------------
echo "--- .deb (cargo-deb) ---"
if command -v cargo-deb >/dev/null 2>&1; then
  cargo deb -p flamix --no-build -o "$DIST/"
else
  echo "warning: cargo-deb not installed; skipping .deb" >&2
fi

# --- .rpm (cargo-generate-rpm) ----------------------------------------------
echo "--- .rpm (cargo-generate-rpm) ---"
if command -v cargo-generate-rpm >/dev/null 2>&1; then
  cargo generate-rpm -p app/flamix -o "$DIST/flamix-${VER}-1.${RPM_ARCH}.rpm"
else
  echo "warning: cargo-generate-rpm not installed; skipping .rpm" >&2
fi

# --- AppImage ----------------------------------------------------------------
echo "--- AppImage ---"
TOOLS="${FLAMIX_TOOLS:-${XDG_CACHE_HOME:-$HOME/.cache}/flamix}"
mkdir -p "$TOOLS"
APPIMAGETOOL="$TOOLS/appimagetool-${AIT_ARCH}.AppImage"
if [ ! -x "$APPIMAGETOOL" ]; then
  echo "downloading appimagetool-${AIT_ARCH}.AppImage -> $APPIMAGETOOL"
  curl -fSL --retry 3 --retry-delay 2 --connect-timeout 20 -o "$APPIMAGETOOL" \
    "https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-${AIT_ARCH}.AppImage"
  chmod +x "$APPIMAGETOOL"
fi

APPDIR="$DIST/Flamix.AppDir"
rm -rf "$APPDIR"
mkdir -p "$APPDIR/usr/bin" "$APPDIR/usr/lib/flamix/cores" "$APPDIR/usr/share/applications"
cp "$BIN" "$APPDIR/usr/bin/flamix"
cp "$STAGE/cores/"* "$APPDIR/usr/lib/flamix/cores/"
cp "$STAGE/flamix.desktop" "$APPDIR/flamix.desktop"
cp "$STAGE/flamix.desktop" "$APPDIR/usr/share/applications/flamix.desktop"
cp "$STAGE/icons/256.png" "$APPDIR/flamix.png"
for size in 16 32 48 64 128 256 512; do
  mkdir -p "$APPDIR/usr/share/icons/hicolor/${size}x${size}/apps"
  cp "$STAGE/icons/${size}.png" "$APPDIR/usr/share/icons/hicolor/${size}x${size}/apps/flamix.png"
done

cat > "$APPDIR/AppRun" <<'EOF'
#!/bin/sh
SELF="$(readlink -f "$0")"
HERE="${SELF%/*}"
export PATH="${HERE}/usr/bin:${PATH}"
exec "${HERE}/usr/bin/flamix" "$@"
EOF
chmod +x "$APPDIR/AppRun"

ARCH="$AIT_ARCH" "$APPIMAGETOOL" --appimage-extract-and-run --no-appstream \
  "$APPDIR" "$DIST/flamix-${VER}-linux-${TARBALL_ARCH}.AppImage"

# --- summary -----------------------------------------------------------------
echo
echo "=== produced files ==="
ls -lh "$DIST"/*.deb "$DIST"/*.rpm "$DIST"/*.tar.gz "$DIST"/*.AppImage "$DIST"/PKGBUILD 2>/dev/null || true

#!/usr/bin/env bash
# Build the macOS universal .app bundle and .dmg.
#
#   dist/Flamix.app
#   dist/flamix-<ver>-macos-universal.dmg
#
# Used by .github/workflows/release.yml (macos-latest). All cores are lipo'd
# into universal binaries; if that is not possible arm64 cores are used.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

VER="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n1)"
[ -n "$VER" ] || { echo "error: cannot read workspace version from Cargo.toml" >&2; exit 1; }

TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/target}"
DIST="$ROOT/dist"
mkdir -p "$DIST"

echo "=== flamix $VER — macOS universal ==="

echo "--- build binaries ---"
cargo build --release -p flamix --target aarch64-apple-darwin
cargo build --release -p flamix --target x86_64-apple-darwin

ARM_BIN="$TARGET_DIR/aarch64-apple-darwin/release/flamix"
X86_BIN="$TARGET_DIR/x86_64-apple-darwin/release/flamix"
[ -x "$ARM_BIN" ] && [ -x "$X86_BIN" ] || { echo "error: build outputs missing" >&2; exit 1; }

echo "--- lipo executable ---"
UNIV_BIN="$DIST/Flamix"
lipo -create -output "$UNIV_BIN" "$ARM_BIN" "$X86_BIN"

echo "--- fetch cores ---"
scripts/fetch-cores.sh macos amd64 "$DIST/cores-amd64"
scripts/fetch-cores.sh macos arm64 "$DIST/cores-arm64"

echo "--- lipo cores ---"
CORES="$DIST/macos-cores"
rm -rf "$CORES"
mkdir -p "$CORES"
for bin in xray sing-box; do
    if lipo -create -output "$CORES/$bin" "$DIST/cores-amd64/$bin" "$DIST/cores-arm64/$bin" 2>/dev/null; then
        echo "  lipo: $bin (universal)"
    else
        echo "  lipo failed for $bin; falling back to arm64" >&2
        cp "$DIST/cores-arm64/$bin" "$CORES/$bin"
    fi
done
cp "$DIST/cores-arm64/geoip.dat" "$CORES/geoip.dat"
cp "$DIST/cores-arm64/geosite.dat" "$CORES/geosite.dat"

echo "--- icons ---"
ICONS="$DIST/macos-icons"
rm -rf "$ICONS"
bash packaging/make-icons.sh "$ICONS" 16 32 64 128 256 512 1024

ICONSET="$DIST/flamix.iconset"
rm -rf "$ICONSET"
mkdir -p "$ICONSET"
cp "$ICONS/16.png" "$ICONSET/icon_16x16.png"
cp "$ICONS/32.png" "$ICONSET/icon_16x16@2x.png"
cp "$ICONS/32.png" "$ICONSET/icon_32x32.png"
cp "$ICONS/64.png" "$ICONSET/icon_32x32@2x.png"
cp "$ICONS/128.png" "$ICONSET/icon_128x128.png"
cp "$ICONS/256.png" "$ICONSET/icon_128x128@2x.png"
cp "$ICONS/256.png" "$ICONSET/icon_256x256.png"
cp "$ICONS/512.png" "$ICONSET/icon_256x256@2x.png"
cp "$ICONS/512.png" "$ICONSET/icon_512x512.png"
cp "$ICONS/1024.png" "$ICONSET/icon_512x512@2x.png"

echo "--- assemble Flamix.app ---"
APP="$DIST/Flamix.app"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources/cores"
cp "$UNIV_BIN" "$APP/Contents/MacOS/Flamix"
cp "$CORES/"* "$APP/Contents/Resources/cores/"
chmod +x "$APP/Contents/MacOS/Flamix"
iconutil -c icns "$ICONSET" -o "$APP/Contents/Resources/flamix.icns"
sed "s/@@VERSION@@/$VER/g" packaging/macos/Info.plist > "$APP/Contents/Info.plist"

echo "--- ad-hoc sign ---"
codesign --force --deep --sign - "$APP"

echo "--- dmg ---"
DMG_STAGE="$DIST/dmg"
rm -rf "$DMG_STAGE"
mkdir -p "$DMG_STAGE"
cp -R "$APP" "$DMG_STAGE/"
ln -s /Applications "$DMG_STAGE/Applications"
hdiutil create -volname "Flamix" -srcfolder "$DMG_STAGE" -ov -format UDZO \
    "$DIST/flamix-${VER}-macos-universal.dmg"

echo
echo "=== produced files ==="
ls -lh "$DIST/flamix-${VER}-macos-universal.dmg"

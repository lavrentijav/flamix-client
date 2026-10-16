#!/usr/bin/env bash
# Build the Windows portable .zip and the Inno Setup installer (.exe).
#
#   dist/flamix-<ver>-windows-x64.zip
#   dist/flamix-<ver>-windows-x64-setup.exe
#
# Runs under Git Bash on the windows-latest runner (used by release.yml).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

VER="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n1)"
[ -n "$VER" ] || { echo "error: cannot read workspace version from Cargo.toml" >&2; exit 1; }

TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/target}"
DIST="$ROOT/dist"
mkdir -p "$DIST"

echo "=== flamix $VER — windows/x64 ==="

echo "--- icons ---"
ICONS="$DIST/windows-icons"
rm -rf "$ICONS"
bash packaging/make-icons.sh "$ICONS" 16 32 48 64 128 256

IM=""
if command -v magick >/dev/null 2>&1; then
    IM="magick"
elif command -v convert >/dev/null 2>&1; then
    IM="convert"
fi
[ -n "$IM" ] || { echo "error: ImageMagick not found" >&2; exit 1; }
echo "  building flamix.ico with $IM"
"$IM" "$ICONS/16.png" "$ICONS/32.png" "$ICONS/48.png" "$ICONS/64.png" \
    "$ICONS/128.png" "$ICONS/256.png" "$DIST/flamix.ico"

# build.rs embeds this into flamix.exe
mkdir -p app/flamix/assets/app-icon
cp "$DIST/flamix.ico" app/flamix/assets/app-icon/flamix.ico

echo "--- build release binary ---"
cargo build --release -p flamix
BIN="$TARGET_DIR/release/flamix.exe"
[ -x "$BIN" ] || { echo "error: binary not found at $BIN" >&2; exit 1; }

echo "--- fetch cores ---"
scripts/fetch-cores.sh windows amd64 "$DIST/cores"

echo "--- portable zip ---"
STAGE_NAME="flamix-${VER}-windows-x64"
STAGE="$DIST/$STAGE_NAME"
rm -rf "$STAGE"
mkdir -p "$STAGE/cores"
cp "$BIN" "$STAGE/flamix.exe"
cp "$DIST/cores/"* "$STAGE/cores/"
cp "$DIST/flamix.ico" "$STAGE/flamix.ico"
cp packaging/windows/README.txt "$STAGE/README.txt"

ZIP="$DIST/${STAGE_NAME}.zip"
rm -f "$ZIP"
if ! (cd "$DIST" && tar -a -c -f "$ZIP" "$STAGE_NAME" 2>/dev/null); then
    echo "  tar -a failed, trying 7z" >&2
    (cd "$DIST" && 7z a -tzip "$ZIP" "$STAGE_NAME" >/dev/null)
fi

echo "--- Inno Setup installer ---"
ISCC="${ISCC:-}"
if [ -z "$ISCC" ]; then
    for cand in "/c/Program Files (x86)/Inno Setup 6/ISCC.exe" \
                "/c/Program Files/Inno Setup 6/ISCC.exe"; do
        if [ -x "$cand" ]; then ISCC="$cand"; break; fi
    done
fi
[ -n "$ISCC" ] || ISCC="$(command -v iscc || true)"
[ -n "$ISCC" ] || { echo "error: ISCC.exe (Inno Setup) not found" >&2; exit 1; }

# MSYS_NO_PATHCONV / MSYS2_ARG_CONV_EXCL stop Git Bash from rewriting the Inno Setup
# `/D...` switches into Windows paths (which ISCC then reads as extra script filenames).
MSYS_NO_PATHCONV=1 MSYS2_ARG_CONV_EXCL='*' "$ISCC" \
    "/DAppVersion=$VER" \
    "/DSourceDir=$(cygpath -w "$STAGE")" \
    "/DOutputDir=$(cygpath -w "$DIST")" \
    "/DIconFile=$(cygpath -w "$DIST/flamix.ico")" \
    "$(cygpath -w packaging/windows/flamix.iss)"

echo
echo "=== produced files ==="
ls -lh "$ZIP" "$DIST/${STAGE_NAME}-setup.exe"

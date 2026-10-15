#!/usr/bin/env bash
# Generate app icon bitmaps for packaging.
#
# Usage: packaging/make-icons.sh <out-dir> [size ...]
# Default sizes: 16 32 48 64 128 256 512 1024
#
# Preference order:
#   1. committed PNGs in app/flamix/assets/app-icon/<size>.png
#   2. rendering app/flamix/assets/app-icon.svg (rsvg-convert / convert / magick)
#   3. a generated placeholder (never fails, T06 assets may not be present yet)
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUT="$1"
shift
SIZES=("$@")
[ ${#SIZES[@]} -gt 0 ] || SIZES=(16 32 48 64 128 256 512 1024)

SRC="$ROOT/app/flamix/assets/app-icon"
mkdir -p "$OUT"

all_png=1
for size in "${SIZES[@]}"; do
    [ -f "$SRC/$size.png" ] || all_png=0
done

if [ "$all_png" = 1 ]; then
    for size in "${SIZES[@]}"; do cp -f "$SRC/$size.png" "$OUT/$size.png"; done
    echo "icons: copied from $SRC"
    exit 0
fi

SVG="$SRC/app-icon.svg"
if [ -f "$SVG" ]; then
    if command -v rsvg-convert >/dev/null 2>&1; then
        for size in "${SIZES[@]}"; do rsvg-convert -w "$size" -h "$size" "$SVG" -o "$OUT/$size.png"; done
        echo "icons: rendered from $SVG (rsvg-convert)"
        exit 0
    elif command -v magick >/dev/null 2>&1; then
        for size in "${SIZES[@]}"; do magick -background none -resize "${size}x${size}" "$SVG" "$OUT/$size.png"; done
        echo "icons: rendered from $SVG (magick)"
        exit 0
    elif command -v convert >/dev/null 2>&1; then
        for size in "${SIZES[@]}"; do convert -background none -resize "${size}x${size}" "$SVG" "$OUT/$size.png"; done
        echo "icons: rendered from $SVG (convert)"
        exit 0
    fi
fi

echo "icons: no committed PNGs and no SVG renderer; generating placeholders" >&2
python3 - "$OUT" "${SIZES[@]}" <<'PY'
import struct
import sys
import zlib

out = sys.argv[1]
sizes = [int(x) for x in sys.argv[2:]]


def make_png(size, path):
    width = height = size
    raw = bytearray()
    for y in range(height):
        t = y / max(1, height - 1)
        r = int(0xC8 + (0x7A - 0xC8) * t)
        g = int(0x86 + (0x4A - 0x86) * t)
        b = int(0x2E + (0x12 - 0x2E) * t)
        raw.append(0)  # filter type 0
        raw.extend(bytes((r, g, b, 255)) * width)

    def chunk(kind, data):
        body = struct.pack(">I", len(data)) + kind + data
        crc = zlib.crc32(kind + data) & 0xFFFFFFFF
        return body + struct.pack(">I", crc)

    ihdr = struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0)
    with open(path, "wb") as f:
        f.write(b"\x89PNG\r\n\x1a\n")
        f.write(chunk(b"IHDR", ihdr))
        f.write(chunk(b"IDAT", zlib.compress(bytes(raw), 9)))
        f.write(chunk(b"IEND", b""))


for size in sizes:
    make_png(size, f"{out}/{size}.png")
print(f"icons: generated {len(sizes)} placeholder PNGs")
PY

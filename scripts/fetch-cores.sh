#!/usr/bin/env bash
# Fetch pinned proxy cores (Xray-core, sing-box, mihomo) and geo data into <out-dir>.
#
# Usage: scripts/fetch-cores.sh <os> <arch> <out-dir>
#   os   : linux | macos | windows
#   arch : amd64 | arm64
#
# Produces in <out-dir>: xray[.exe], sing-box[.exe], mihomo[.exe], geoip.dat, geosite.dat
#
# SHA256 is verified whenever the upstream release publishes a digest:
#   - Xray-core ships `<asset>.dgst` files (parsed below)
#   - sing-box does not publish per-asset digests (skipped)
#   - mihomo (MetaCubeX/mihomo) assets are checked against the GitHub release API `digest`
#     (sha256) when the API answers; otherwise the check is skipped with a note
#   - Loyalsoldier/v2ray-rules-dat ships `<file>.sha256sum`
# The geo databases from Xray are overridden by Loyalsoldier's (better RU lists).
set -euo pipefail

# --- pinned upstream versions ------------------------------------------------
XRAY_VERSION="v26.3.27"
SINGBOX_VERSION="1.14.3"
MIHOMO_VERSION="v1.19.33"
GEODAT_TAG="202610142206"

usage() { echo "usage: $0 <linux|macos|windows> <amd64|arm64> <out-dir>" >&2; exit 2; }
[ $# -eq 3 ] || usage

OS="$1"
ARCH="$2"
OUT="$3"

case "$OS" in
  linux | macos | windows) ;;
  *) echo "error: unsupported os '$OS' (linux|macos|windows)" >&2; exit 2 ;;
esac
case "$ARCH" in
  amd64 | arm64) ;;
  *) echo "error: unsupported arch '$ARCH' (amd64|arm64)" >&2; exit 2 ;;
esac

if [ "$OS" = "windows" ]; then EXE=".exe"; else EXE=""; fi

# --- upstream asset names ----------------------------------------------------
case "$OS/$ARCH" in
  linux/amd64)
    XRAY_ASSET="Xray-linux-64.zip"
    SINGBOX_ASSET="sing-box-${SINGBOX_VERSION}-linux-amd64.tar.gz"
    MIHOMO_ASSET="mihomo-linux-amd64-${MIHOMO_VERSION}.gz"
    ;;
  linux/arm64)
    XRAY_ASSET="Xray-linux-arm64-v8a.zip"
    SINGBOX_ASSET="sing-box-${SINGBOX_VERSION}-linux-arm64.tar.gz"
    MIHOMO_ASSET="mihomo-linux-arm64-${MIHOMO_VERSION}.gz"
    ;;
  macos/amd64)
    XRAY_ASSET="Xray-macos-64.zip"
    SINGBOX_ASSET="sing-box-${SINGBOX_VERSION}-darwin-amd64.tar.gz"
    MIHOMO_ASSET="mihomo-darwin-amd64-${MIHOMO_VERSION}.gz"
    ;;
  macos/arm64)
    XRAY_ASSET="Xray-macos-arm64-v8a.zip"
    SINGBOX_ASSET="sing-box-${SINGBOX_VERSION}-darwin-arm64.tar.gz"
    MIHOMO_ASSET="mihomo-darwin-arm64-${MIHOMO_VERSION}.gz"
    ;;
  windows/amd64)
    XRAY_ASSET="Xray-windows-64.zip"
    SINGBOX_ASSET="sing-box-${SINGBOX_VERSION}-windows-amd64.zip"
    MIHOMO_ASSET="mihomo-windows-amd64-${MIHOMO_VERSION}.zip"
    ;;
  windows/arm64)
    XRAY_ASSET="Xray-windows-arm64-v8a.zip"
    SINGBOX_ASSET="sing-box-${SINGBOX_VERSION}-windows-arm64.zip"
    MIHOMO_ASSET="mihomo-windows-arm64-${MIHOMO_VERSION}.zip"
    ;;
esac

XRAY_URL="https://github.com/XTLS/Xray-core/releases/download/${XRAY_VERSION}/${XRAY_ASSET}"
SINGBOX_URL="https://github.com/SagerNet/sing-box/releases/download/v${SINGBOX_VERSION}/${SINGBOX_ASSET}"
MIHOMO_URL="https://github.com/MetaCubeX/mihomo/releases/download/${MIHOMO_VERSION}/${MIHOMO_ASSET}"
GEODAT_BASE="https://github.com/Loyalsoldier/v2ray-rules-dat/releases/download/${GEODAT_TAG}"

# --- helpers -----------------------------------------------------------------
mkdir -p "$OUT"
TMP="$(mktemp -d "${TMPDIR:-/tmp}/flamix-cores.XXXXXX")"
trap 'rm -rf "$TMP"' EXIT

download() { # <url> <dest>
  echo "  fetch $(basename "$1")"
  curl -fSL --retry 3 --retry-delay 2 --connect-timeout 20 -o "$2" "$1"
}

sha256() { # <file> -> hex digest
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$1" | awk '{print $1}'
  else
    python3 -c 'import hashlib,sys; print(hashlib.sha256(open(sys.argv[1],"rb").read()).hexdigest())' "$1"
  fi
}

verify_sha256() { # <file> <expected>
  local actual
  actual="$(sha256 "$1")"
  if [ "$actual" != "$2" ]; then
    echo "error: SHA256 mismatch for $(basename "$1")" >&2
    echo "  expected: $2" >&2
    echo "  actual:   $actual" >&2
    exit 1
  fi
  echo "  sha256 ok: $(basename "$1")"
}

extract_zip() { # <archive> <dest-dir>
  mkdir -p "$2"
  if command -v unzip >/dev/null 2>&1; then
    unzip -oq "$1" -d "$2"
  elif command -v python3 >/dev/null 2>&1; then
    python3 -m zipfile -e "$1" "$2"
  elif command -v python >/dev/null 2>&1; then
    python -m zipfile -e "$1" "$2"
  else
    tar -xf "$1" -C "$2"
  fi
}

# --- Xray-core (also brings geoip.dat/geosite.dat) ---------------------------
echo "== Xray-core ${XRAY_VERSION} (${OS}/${ARCH}) =="
download "$XRAY_URL" "$TMP/$XRAY_ASSET"
XRAY_SHA="$(curl -fSL --retry 3 --connect-timeout 20 "$XRAY_URL.dgst" 2>/dev/null \
  | sed -n 's/^SHA2-256=[[:space:]]*//p' | tr -d '\r')" || true
[ -n "$XRAY_SHA" ] || { echo "error: could not parse SHA2-256 from ${XRAY_URL}.dgst" >&2; exit 1; }
verify_sha256 "$TMP/$XRAY_ASSET" "$XRAY_SHA"
extract_zip "$TMP/$XRAY_ASSET" "$TMP/xray"
cp -f "$TMP/xray/xray${EXE}" "$OUT/xray${EXE}"
cp -f "$TMP/xray/geoip.dat" "$OUT/geoip.dat"
cp -f "$TMP/xray/geosite.dat" "$OUT/geosite.dat"

# --- sing-box (no per-asset digests published upstream) ----------------------
echo "== sing-box ${SINGBOX_VERSION} (${OS}/${ARCH}) =="
download "$SINGBOX_URL" "$TMP/$SINGBOX_ASSET"
case "$SINGBOX_ASSET" in
  *.zip) extract_zip "$TMP/$SINGBOX_ASSET" "$TMP/singbox" ;;
  *)
    mkdir -p "$TMP/singbox"
    tar -xzf "$TMP/$SINGBOX_ASSET" -C "$TMP/singbox"
    ;;
esac
SINGBOX_BIN="$(find "$TMP/singbox" -type f -name "sing-box${EXE}" | head -n 1)"
[ -n "$SINGBOX_BIN" ] || { echo "error: sing-box${EXE} not found in archive" >&2; exit 1; }
cp -f "$SINGBOX_BIN" "$OUT/sing-box${EXE}"

# --- mihomo (Clash.Meta): .gz = a single binary, .zip (Windows) = binary inside -------
echo "== mihomo ${MIHOMO_VERSION} (${OS}/${ARCH}) =="
download "$MIHOMO_URL" "$TMP/$MIHOMO_ASSET"
MIHOMO_SHA="$(curl -fsSL --retry 2 --connect-timeout 20 \
    "https://api.github.com/repos/MetaCubeX/mihomo/releases/tags/${MIHOMO_VERSION}" 2>/dev/null \
  | python3 -c 'import json,sys
asset=sys.argv[1]
try:
    for a in json.load(sys.stdin).get("assets", []):
        if a.get("name") == asset and str(a.get("digest","")).startswith("sha256:"):
            print(a["digest"][7:])
except Exception:
    pass' "$MIHOMO_ASSET" 2>/dev/null)" || true
if [ -n "$MIHOMO_SHA" ]; then
  verify_sha256 "$TMP/$MIHOMO_ASSET" "$MIHOMO_SHA"
else
  echo "  note: no published sha256 for ${MIHOMO_ASSET}; skipped"
fi
case "$MIHOMO_ASSET" in
  *.zip)
    extract_zip "$TMP/$MIHOMO_ASSET" "$TMP/mihomo"
    MIHOMO_BIN="$(find "$TMP/mihomo" -type f -name "mihomo*.exe" | head -n 1)"
    ;;
  *)
    mkdir -p "$TMP/mihomo"
    gunzip -c "$TMP/$MIHOMO_ASSET" > "$TMP/mihomo/mihomo"
    MIHOMO_BIN="$TMP/mihomo/mihomo"
    ;;
esac
[ -n "$MIHOMO_BIN" ] || { echo "error: mihomo binary not found in archive" >&2; exit 1; }
cp -f "$MIHOMO_BIN" "$OUT/mihomo${EXE}"

# --- geo databases: Loyalsoldier overrides Xray's (better RU lists) ----------
echo "== geoip.dat/geosite.dat (Loyalsoldier ${GEODAT_TAG}) =="
for name in geoip.dat geosite.dat; do
  download "${GEODAT_BASE}/${name}" "$TMP/$name"
  GEODAT_SHA="$(curl -fSL --retry 3 --connect-timeout 20 "${GEODAT_BASE}/${name}.sha256sum" 2>/dev/null \
    | awk '{print $1}' | tr -d '\r')" || true
  [ -n "$GEODAT_SHA" ] || { echo "error: could not fetch ${name}.sha256sum" >&2; exit 1; }
  verify_sha256 "$TMP/$name" "$GEODAT_SHA"
  cp -f "$TMP/$name" "$OUT/$name"
done

chmod +x "$OUT/xray${EXE}" "$OUT/sing-box${EXE}" "$OUT/mihomo${EXE}"
echo "cores written to $OUT:"
ls -l "$OUT"

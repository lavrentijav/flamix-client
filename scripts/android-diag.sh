#!/usr/bin/env bash
# Emulator diagnostics for the phone app: normal launch, screenshot, tap Connect, accept the system
# VPN consent dialog, wait, then collect screenshots, logcat and the in-app log.
# Usage: scripts/android-diag.sh <apk> <outdir>   (an emulator must be running and visible to adb)
set -u
APK="${1:?apk path}"
OUT="${2:?output dir}"
PKG=online.flamix.client
mkdir -p "$OUT"
step() { echo "=== $*"; }
shot() { adb exec-out screencap -p > "$OUT/$1.png"; ls -l "$OUT/$1.png"; }
TAGS='FATAL|AndroidRuntime|Flamix|FlamixCore|TunnelService|FlamixVpn|NativeTunnel|tun2proxy|flamix|DEBUG|libc|VpnService|Vpn'

step "install"; adb install -r "$APK" || exit 1
adb shell pm grant "$PKG" android.permission.POST_NOTIFICATIONS 2>&1 || true

# Seed one local vless server (no subscription needed) so the Connect button has a target.
step "seed state.json"
SEED='{"version":1,"selected_server":"diag1","local_servers":[{"id":"diag1","name":"Diag server","address":"203.0.113.5","port":443,"credentials":{"protocol":"vless","uuid":"11111111-1111-1111-1111-111111111111"},"transport":{"network":"tcp"},"security":{"type":"none"}}]}'
adb shell "run-as $PKG sh -c 'mkdir -p files; cat > files/state.json'" <<<"$SEED" || echo "run-as seed failed"
adb shell "run-as $PKG ls -l files"

adb shell settings put global hide_error_dialogs 1 || true   # a launcher ANR dialog once swallowed the tap
adb shell am force-stop "$PKG"
adb shell "run-as $PKG rm -f files/flamix.log files/flamix.log.1" || true
adb logcat -c
step "launch normally (launcher intent)"
adb shell monkey -p "$PKG" -c android.intent.category.LAUNCHER 1
sleep 15
shot 1-launched
adb shell dumpsys window | grep -E "mCurrentFocus|mFocusedApp" | head -3

step "screen size / density"
SIZE=$(adb shell wm size | tail -1 | grep -oE '[0-9]+x[0-9]+' | tail -1)
DENS=$(adb shell wm density | tail -1 | grep -oE '[0-9]+' | tail -1)
W=${SIZE%x*}; H=${SIZE#*x}
echo "size=$SIZE density=$DENS"
# Connect button centre: top bar 56dp + 16dp padding + half of the 152dp button.
TAPX=$((W / 2))
TAPY=$(( (56 + 16 + 76) * DENS / 160 ))
echo "tap connect at $TAPX,$TAPY"
adb shell input tap "$TAPX" "$TAPY"
sleep 4
shot 2-after-connect-tap

step "accept VPN consent dialog if shown"
for i in 1 2 3; do
  adb shell uiautomator dump /sdcard/ui.xml >/dev/null 2>&1
  adb pull /sdcard/ui.xml "$OUT/ui-$i.xml" >/dev/null 2>&1
  # Find the OK button of com.android.vpndialogs (resource-id android:id/button1, text OK).
  xy=$(python3 - "$OUT/ui-$i.xml" <<'PY'
import re, sys
try:
    xml = open(sys.argv[1], encoding="utf-8", errors="replace").read()
except OSError:
    sys.exit(0)
for m in re.finditer(r'<node [^>]*>', xml):
    n = m.group(0)
    if 'vpndialogs' in n or 'android:id/button1' in n:
        if 'android:id/button1' in n or 'text="OK"' in n:
            b = re.search(r'bounds="\[(\d+),(\d+)\]\[(\d+),(\d+)\]"', n)
            if b:
                x1, y1, x2, y2 = map(int, b.groups())
                print((x1 + x2) // 2, (y1 + y2) // 2)
                break
PY
)
  if [ -n "$xy" ]; then
    echo "consent OK button at $xy"; shot "3-consent-dialog"
    adb shell input tap $xy
    break
  fi
  echo "no consent dialog yet (attempt $i)"; sleep 3
done
sleep 30
shot 4-after-consent

step "Settings tab screenshot"
adb shell input tap $((W * 5 / 6)) $((H - 28 * DENS / 160))
sleep 2
shot 5-settings-tab

step "VPN state"
adb shell dumpsys connectivity | grep -iE "VPN|tun" | head -20
adb shell "ps -A" | grep -E "libsingbox|flamix" || echo "no libsingbox/flamix process"
adb shell dumpsys activity services online.flamix.client | head -20

step "in-app log (files/flamix.log)"
adb shell "run-as $PKG cat files/flamix.log" > "$OUT/flamix.log" 2>&1 || true
tail -n 80 "$OUT/flamix.log"
adb shell "run-as $PKG cat files/singbox-vpn.json" > "$OUT/singbox-vpn.json" 2>&1 || true

step "logcat"
adb logcat -d -v threadtime > "$OUT/logcat-full.txt" 2>&1
grep -E "$TAGS" "$OUT/logcat-full.txt" > "$OUT/logcat.txt" || true
wc -l "$OUT/logcat.txt"
tail -n 150 "$OUT/logcat.txt"
exit 0

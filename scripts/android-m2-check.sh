#!/usr/bin/env bash
# M2 emulator check: start the app, trigger the core via the debug intent, verify process + port 10808.
# Usage: scripts/android-m2-check.sh <apk>   (an emulator must be running and visible to adb)
set -u
APK="${1:?apk path}"
PKG=online.flamix.client
fail=0
step() { echo "=== $*"; }

step "install"; adb install -r "$APK" || exit 1
step "launch app and start core"
adb logcat -c
adb shell am start -n "$PKG/online.flamix.MainActivity" --es core start
sleep 15

step "logcat (FlamixCore)"; adb logcat -d -s FlamixCore:V AndroidRuntime:E | tail -30
step "nativeLibraryDir"; adb shell "pm path $PKG; ls -l \$(dirname \$(pm path $PKG | head -1 | cut -d: -f2))/lib/*/ 2>&1" || true

step "core process alive?"
if adb shell "ps -A" | grep -q libsingbox; then echo "OK: libsingbox running"; adb shell "ps -A" | grep libsingbox
else echo "FAIL: libsingbox not running"; fail=1; fi

step "port 10808 on device"
if adb shell "toybox nc -z 127.0.0.1 10808" ; then echo "OK: toybox nc -z"
elif adb shell "echo | toybox nc -w 2 127.0.0.1 10808; echo rc=\$?" | tee /dev/stderr | grep -q 'rc=0'; then echo "OK: nc connect"
else echo "FAIL: port 10808 not answering"; fail=1; fi
adb shell "cat /proc/net/tcp /proc/net/tcp6 | grep -i ':2A38'" && echo "listening socket 0x2A38 (10808) present"

step "forwarded curl via proxy (informational)"
adb forward tcp:10808 tcp:10808
echo "-- direct internet check from the host (no proxy)"
curl -sS -m 15 -o /dev/null -w 'host http=%{http_code}\n' http://example.com || echo "host curl failed"
echo "-- IP literal via proxy (no DNS)"
curl -sS -m 20 -x socks5h://127.0.0.1:10808 -o /dev/null -w 'proxy ip-literal http=%{http_code}\n' http://1.1.1.1/ || echo "proxy ip-literal failed (not fatal)"
echo "-- domain via proxy"
curl -sS -m 20 -x socks5h://127.0.0.1:10808 -o /dev/null -w 'proxy domain http=%{http_code}\n' http://example.com || echo "proxy domain failed (not fatal)"

step "stop core"; adb shell am start -n "$PKG/online.flamix.MainActivity" --es core stop; sleep 5
adb shell "ps -A" | grep libsingbox && echo "WARN: still running after stop" || echo "OK: stopped"
exit $fail

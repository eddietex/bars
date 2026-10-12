#!/usr/bin/env bash
# Smoke test on a macOS runner: runs the debug build and checks the CLI,
# single instance, windows and overlay. Screenshots and logs land in smoke/
# (uploaded as a CI artifact). Typing needs Accessibility, which a runner
# can't grant, so pasting is only tested on Windows.
set -euo pipefail
exe=target/debug/bars
mkdir -p smoke

echo "== doctor"
"$exe" --doctor | tee smoke/doctor.txt
grep -q "diagnostics" smoke/doctor.txt

echo "== launch"
RUST_LOG=info "$exe" > smoke/bars.out 2> smoke/bars.log &
app=$!
sleep 12
kill -0 "$app" || { echo "bars exited early"; cat smoke/bars.log; exit 1; }
screencapture -x smoke/1-setup-assistant.png || true

echo "== second launch forwards to the running instance"
"$exe"
sleep 1
running=$(pgrep -x bars | wc -l | tr -d ' ')
[ "$running" = 1 ] || { echo "expected 1 bars process, found $running"; exit 1; }

echo "== settings"
"$exe" --settings
sleep 3
screencapture -x smoke/2-settings.png || true

echo "== overlay"
"$exe" --toggle
sleep 2
screencapture -x smoke/3-overlay.png || true
"$exe" --cancel
sleep 1

echo "== quit"
"$exe" --quit
for _ in $(seq 30); do kill -0 "$app" 2>/dev/null || break; sleep 0.5; done
if kill -0 "$app" 2>/dev/null; then echo "bars did not quit"; exit 1; fi

echo "== log"
cat smoke/bars.log
echo "smoke test passed"

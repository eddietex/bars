#!/usr/bin/env bash
# Smoke test on a macOS runner: runs the debug build and checks the CLI,
# single instance, windows, overlay and pasting into TextEdit (the runner
# grants Accessibility). Screenshots and logs land in smoke/ (uploaded as a
# CI artifact).
set -euo pipefail
exe=target/debug/bars
mkdir -p smoke

# One history entry for the paste test.
data="$HOME/Library/Application Support/bars"
mkdir -p "$data"
printf '{"id":1,"at":"%s","text":"Hello from the Bars smoke test","model":"test","audio_secs":1.0,"infer_ms":1,"words":6}\n' \
  "$(date -u +%Y-%m-%dT%H:%M:%S+00:00)" > "$data/history.jsonl"

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

echo "== paste into another app"
# A blank TextEdit document instead of the open panel.
defaults write com.apple.TextEdit NSShowAppCentricOpenPanelInsteadOfUntitledFile -bool false
open -a TextEdit
sleep 5
"$exe" --paste-last
sleep 3
screencapture -x smoke/4-pasted.png || true
kill -0 "$app" 2>/dev/null || { echo "bars crashed while pasting"; cat smoke/bars.log; exit 1; }
grep -Eq "inserted [0-9]+ characters" smoke/bars.log || { echo "paste-last inserted nothing"; cat smoke/bars.log; exit 1; }
osascript -e 'tell application "TextEdit" to quit saving no' >/dev/null 2>&1 || true

echo "== quit"
"$exe" --quit
for _ in $(seq 30); do kill -0 "$app" 2>/dev/null || break; sleep 0.5; done
if kill -0 "$app" 2>/dev/null; then echo "bars did not quit"; exit 1; fi

echo "== log"
cat smoke/bars.log
echo "smoke test passed"

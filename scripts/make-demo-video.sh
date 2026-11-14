#!/usr/bin/env bash
# scripts/make-demo-video.sh
# Records a narrated-style product demo of zniff on macOS and renders
# screenshots/zniff_demo.mp4 (1920x1080, 60 fps).
#
# How it differs from a plain screen capture:
#   * the cursor glides along eased paths and text is typed key by key;
#   * the storyboard below *is* the final pacing (no dead time from tooling);
#   * a virtual camera zooms into whatever the scene is about;
#   * chapter captions (EN + 中文), a backdrop with shadow, intro and outro cards.
#
# Needs: macOS 14+, ffmpeg, Xcode command line tools (swiftc), Screen Recording and
# Accessibility permission for the terminal. It takes over mouse and keyboard
# for about a minute and temporarily switches the system appearance.
#
# Env: ZNIFF_BIN=<path> to skip building, DEMO_OUT=<file.mp4>, DEMO_KEEP=1 to keep the raw recording.

set -euo pipefail
source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/demo-video/lib.sh"

OUT="${DEMO_OUT:-$REPO_ROOT/screenshots/zniff_demo.mp4}"
RAW="$WORK/raw.mov"
export DRV_LOG="$WORK/driver.log"
rm -f "$RAW" "$RAW.start" "$DRV_LOG"

ORIGINAL_DARK="$(osascript -e 'tell application "System Events" to tell appearance preferences to get dark mode' 2>/dev/null || echo true)"
trap demo_cleanup EXIT INT TERM
osascript -e 'tell application "System Events" to tell appearance preferences to set dark mode to true' >/dev/null 2>&1 || true

demo_build_driver
swiftc -O "$DEMO_DIR/assets.swift" -o "$WORK/assets" 2>/dev/null
swiftc -O "$DEMO_DIR/recorder.swift" -o "$WORK/recorder" 2>/dev/null
demo_find_binary
demo_start_mock
demo_write_config
demo_launch_app
caffeinate -d -t 300 &
CAFF_PID=$!

d() { "$DRV" "$@"; }
mark() { d mark "$@"; }
cam() { d cam "$@"; }
row() { echo $((132 + 34 * ($1 - 1) + ${2:-0})); }
hold() { sleep "$1"; }

d move 640 600 0.2
demo_front
echo "==> Recording window $WID..."
# ScreenCaptureKit window capture: only zniff's own pixels end up in the file,
# even if another window is moved on top of it during the recording.
"$WORK/recorder" "$WID" "$RAW" &
REC_PID=$!
sleep 1.5
# Keep zniff frontmost so clicks land on it.
for _ in {1..20}; do
    demo_front
    sleep 0.3
    [ "$(osascript -e 'tell application "System Events" to get name of first process whose frontmost is true' 2>/dev/null)" = "zniff" ] && break
done
sleep 0.8

# ----------------------------------------------------------------- storyboard
mark begin
hold 0.6
mark capture "Capture every request, live" "实时捕获每一个请求"
cam 1.3 470 330 1.4
demo_traffic 0.32
hold 1.6

mark inspect "Readable bodies: JSON, GraphQL, images, Protobuf" "JSON、GraphQL、图片、Protobuf 一目了然"
cam 1.3 800 340 1.0
d click 440 "$(row 5)"
hold 2.8
d click 440 "$(row 2)" 0.5
hold 2.0
d click 440 "$(row 8)" 0.6
hold 2.2
d click 440 "$(row 3)" 0.5
hold 2.2

mark filter "Filter with a tiny query language" "简洁的过滤语法，随打随筛"
cam 1.6 330 260 1.0
d click 260 64
hold 0.3
d type "POST" 9
hold 1.6
d key 0 cmd
d type "status:5xx|304" 11
hold 1.8
d key 0 cmd
d type "body:alice" 11
hold 1.8
d key 0 cmd
d key 51
cam 1 640 360 0.9
hold 0.8

mark sequence "One click to a waterfall & sequence diagram" "一键生成瀑布图与时序图"
d click 1140 20
hold 0.7
d click 987 164 0.6
cam 1.5 965 390 1.0
hold 2.6
d click 809 371 0.6
hold 2.4
d click 1218 78 0.7
cam 1 640 360 0.9
hold 0.6

mark breakpoint "Pause, edit and resume requests" "断点拦截：改完再放行"
req GET "$MOCK/api/v1/checkout/payment" &
BP_PIDS="$!"
sleep 0.35
req POST "$MOCK/api/v1/checkout/confirm" -H 'Content-Type: application/json' -d '{"orderId":"ord_88231"}' &
BP_PIDS="$BP_PIDS $!"
hold 0.6
cam 1.6 860 150 1.0
d move 1080 61 0.9
hold 1.2
d click 1080 61 0.2
hold 1.5
d click 1173 61 0.6
hold 1.6
cam 1 640 360 0.9
hold 0.5

mark rules "Rewrite traffic with 14 rule types" "14 种改写规则：映射、替换、延迟、阻断……"
d click 953 20
cam 1.4 610 310 1.0
hold 1.6
d click 826 340 0.6
hold 1.4
d click 660 340 0.5
hold 1.4
d key 53
cam 1 640 360 0.9
hold 0.6

mark theme "Native app that follows light & dark mode" "原生应用，自动适配深浅色"
d click 440 "$(row 5)" 0.7
hold 1.0
osascript -e 'tell application "System Events" to tell appearance preferences to set dark mode to false' >/dev/null 2>&1 || true
hold 4.5
mark end
# ---------------------------------------------------------------------------

sleep 0.5
kill -INT "$REC_PID" 2>/dev/null || true
wait "$REC_PID" 2>/dev/null || true
unset REC_PID
kill "$CAFF_PID" 2>/dev/null || true
demo_cleanup
trap - EXIT INT TERM

echo "==> Composing video..."
mkdir -p "$(dirname "$OUT")"
python3 "$DEMO_DIR/compose.py" "$RAW" "$DRV_LOG" "$WORK" "$WORK/assets" "$REPO_ROOT/icons/zniff.icns" "$OUT"
[ "${DEMO_KEEP:-0}" = "1" ] || rm -f "$RAW" "$RAW.start"
echo "==> Done: $OUT ($(du -h "$OUT" | cut -f1))"

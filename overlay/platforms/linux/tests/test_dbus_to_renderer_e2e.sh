#!/usr/bin/env sh
set -eu

PROJECT_DIRECTORY=$(CDPATH='' cd -- "$(dirname "$0")/../../../.." && pwd)
GOLDEN_IMAGE="$PROJECT_DIRECTORY/overlay/platforms/linux/tests/fixtures/qt-overlay.png"
DAEMON=${KEYMAP_OVERLAY_E2E_DAEMON:-"$PROJECT_DIRECTORY/target/release/keymap-overlay"}
RENDERER=${KEYMAP_OVERLAY_E2E_RENDERER:-"$PROJECT_DIRECTORY/target/release/keymap-overlay-qt"}
TEST_DIRECTORY=$(mktemp -d)
DAEMON_PID=''
RENDERER_PID=''

cleanup() {
  if [ -n "$RENDERER_PID" ]; then
    kill "$RENDERER_PID" 2>/dev/null || true
    wait "$RENDERER_PID" 2>/dev/null || true
  fi
  if [ -n "$DAEMON_PID" ]; then
    kill "$DAEMON_PID" 2>/dev/null || true
    wait "$DAEMON_PID" 2>/dev/null || true
  fi
  rm -rf "$TEST_DIRECTORY"
}
trap cleanup EXIT HUP INT TERM

fail() {
  printf 'D-Bus-to-renderer E2E failure: %s\n' "$1" >&2
  if [ -f "$TEST_DIRECTORY/daemon.log" ]; then
    printf '%s\n' 'Daemon log:' >&2
    sed 's/^/  /' "$TEST_DIRECTORY/daemon.log" >&2
  fi
  if [ -f "$TEST_DIRECTORY/renderer.log" ]; then
    printf '%s\n' 'Renderer log:' >&2
    sed 's/^/  /' "$TEST_DIRECTORY/renderer.log" >&2
  fi
  exit 1
}

get_state() {
  gdbus call --session \
    --dest com.sunaemon.KeymapOverlay \
    --object-path /com/sunaemon/KeymapOverlay \
    --method com.sunaemon.KeymapOverlay.Renderer1.GetState 2>/dev/null
}

wait_for_state() {
  description=$1
  pattern=$2
  attempts=0
  while [ "$attempts" -lt 100 ]; do
    if ! kill -0 "$DAEMON_PID" 2>/dev/null; then
      fail "daemon exited while waiting for $description"
    fi
    state=$(get_state || true)
    case "$state" in
      *"$pattern"*) return 0 ;;
    esac
    attempts=$((attempts + 1))
    sleep 0.05
  done
  fail "timed out waiting for $description; last state: $state"
}

"$DAEMON" --simulate 1:2 \
  >"$TEST_DIRECTORY/daemon.log" 2>&1 &
DAEMON_PID=$!

wait_for_state 'the composed layer to become visible' ', true, '\''{"version":2,"layer":2'
wait_for_state 'the held key metadata' '"label":["E2E"],"held":true'

QT_QPA_PLATFORM=offscreen QT_QUICK_BACKEND=software QT_SCALE_FACTOR=1 \
  KEYMAP_OVERLAY_FORCE_QT=1 \
  KEYMAP_OVERLAY_GOLDEN_OUTPUT="$TEST_DIRECTORY/qt-overlay.png" "$RENDERER" \
  >"$TEST_DIRECTORY/renderer.log" 2>&1 &
RENDERER_PID=$!

attempts=0
while [ ! -s "$TEST_DIRECTORY/qt-overlay.png" ] && [ "$attempts" -lt 100 ]; do
  if ! kill -0 "$RENDERER_PID" 2>/dev/null; then
    fail 'Qt renderer exited before capturing the golden render'
  fi
  attempts=$((attempts + 1))
  sleep 0.05
done
if [ ! -s "$TEST_DIRECTORY/qt-overlay.png" ]; then
  fail 'timed out waiting for the Qt golden render'
fi
if [ "${UPDATE_GOLDEN:-false}" = true ]; then
  cp "$TEST_DIRECTORY/qt-overlay.png" "$GOLDEN_IMAGE"
fi
if ! compare -metric AE -fuzz 2% "$GOLDEN_IMAGE" \
  "$TEST_DIRECTORY/qt-overlay.png" null: 2>"$TEST_DIRECTORY/golden-diff.txt"; then
  difference=$(cat "$TEST_DIRECTORY/golden-diff.txt")
  fail "Qt render differs from the golden image by $difference pixels"
fi

if ! kill -0 "$RENDERER_PID" 2>/dev/null; then
  fail 'Qt renderer exited while consuming the visible state'
fi
if [ -s "$TEST_DIRECTORY/renderer.log" ]; then
  fail 'Qt renderer logged an error while consuming the visible state'
fi

wait_for_state 'the simulated key release to hide the overlay' ", false, '')"
wait_for_state 'the next simulated key press to show the overlay again' ', true, '\''{"version":2,"layer":2'
sleep 0.1

if ! kill -0 "$RENDERER_PID" 2>/dev/null; then
  fail 'Qt renderer exited while consuming state transitions'
fi
if [ -s "$TEST_DIRECTORY/renderer.log" ]; then
  fail 'Qt renderer logged an error while consuming state transitions'
fi

printf '%s\n' 'Linux D-Bus-to-renderer E2E test passed'

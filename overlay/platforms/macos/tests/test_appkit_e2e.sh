#!/usr/bin/env sh
set -eu

PROJECT_DIRECTORY=$(CDPATH='' cd -- "$(dirname "$0")/../../../.." && pwd)
OVERLAY=${KEYMAP_OVERLAY_E2E_OVERLAY:-"$PROJECT_DIRECTORY/target/release/keymap-overlay"}
TEST_DIRECTORY=$(mktemp -d)
OVERLAY_PID=''

cleanup() {
  if [ -n "$OVERLAY_PID" ]; then
    kill "$OVERLAY_PID" 2>/dev/null || true
    wait "$OVERLAY_PID" 2>/dev/null || true
  fi
  rm -rf "$TEST_DIRECTORY"
}
trap cleanup EXIT HUP INT TERM

fail() {
  printf 'AppKit E2E failure: %s\n' "$1" >&2
  if [ -f "$TEST_DIRECTORY/overlay.log" ]; then
    printf '%s\n' 'Overlay log:' >&2
    sed 's/^/  /' "$TEST_DIRECTORY/overlay.log" >&2
  fi
  if [ -f "$TEST_DIRECTORY/state" ]; then
    printf '%s\n' 'Observed AppKit states:' >&2
    sed 's/^/  /' "$TEST_DIRECTORY/state" >&2
  fi
  exit 1
}

wait_for_state() {
  description=$1
  pattern=$2
  count=${3:-1}
  attempts=0
  while [ "$attempts" -lt 100 ]; do
    if ! kill -0 "$OVERLAY_PID" 2>/dev/null; then
      fail "overlay exited while waiting for $description"
    fi
    matches=$(grep -F -c "$pattern" "$TEST_DIRECTORY/state" 2>/dev/null || true)
    matches=${matches:-0}
    if [ "$matches" -ge "$count" ]; then
      return 0
    fi
    attempts=$((attempts + 1))
    sleep 0.05
  done
  fail "timed out waiting for $description"
}

KEYMAP_OVERLAY_E2E_STATE_FILE="$TEST_DIRECTORY/state" \
  "$OVERLAY" --simulate 1:2 \
  >"$TEST_DIRECTORY/overlay.log" 2>&1 &
OVERLAY_PID=$!

wait_for_state 'the composed layer to be attached' \
  'show keyboard=1 layers=[2] size=160x120 subviews=1 native_subviews=5'
wait_for_state 'the simulated release to detach and hide the layer' \
  'hide size=1x1 subviews=0'
wait_for_state 'the next simulated press to attach the layer again' \
  'show keyboard=1 layers=[2] size=160x120 subviews=1 native_subviews=5' 2

if ! kill -0 "$OVERLAY_PID" 2>/dev/null; then
  fail 'overlay exited while processing AppKit state transitions'
fi

printf '%s\n' 'macOS AppKit E2E test passed'

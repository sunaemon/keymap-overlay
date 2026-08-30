#!/usr/bin/env sh
set -eu

PROJECT_DIRECTORY=$(CDPATH='' cd -- "$(dirname "$0")/../../../.." && pwd)
DAEMON=${KEYMAP_OVERLAY_E2E_DAEMON:-"$PROJECT_DIRECTORY/target/release/keymap-overlay"}
VIRTUAL_HID=${KEYMAP_OVERLAY_E2E_VIRTUAL_HID:-"$PROJECT_DIRECTORY/target/virtual-raw-hid"}
VIAL_DEFINITION="$PROJECT_DIRECTORY/model/tests/data/vial-contract.json"
TEST_DIRECTORY=$(mktemp -d)
DAEMON_PID=''
VIRTUAL_HID_PID=''

cleanup() {
  if [ -n "$DAEMON_PID" ]; then
    kill "$DAEMON_PID" 2>/dev/null || true
    wait "$DAEMON_PID" 2>/dev/null || true
  fi
  if [ -n "$VIRTUAL_HID_PID" ]; then
    kill "$VIRTUAL_HID_PID" 2>/dev/null || true
    wait "$VIRTUAL_HID_PID" 2>/dev/null || true
  fi
  rm -rf "$TEST_DIRECTORY"
}
trap cleanup EXIT HUP INT TERM

fail() {
  printf 'HID-to-D-Bus E2E failure: %s\n' "$1" >&2
  for log in virtual-hid daemon; do
    if [ -f "$TEST_DIRECTORY/$log.log" ]; then
      printf '%s log:\n' "$log" >&2
      sed 's/^/  /' "$TEST_DIRECTORY/$log.log" >&2
    fi
  done
  exit 1
}

get_state() {
  gdbus call --session \
    --dest com.sunaemon.KeymapOverlay \
    --object-path /com/sunaemon/KeymapOverlay \
    --method com.sunaemon.KeymapOverlay.Renderer1.GetState 2>/dev/null
}

find_virtual_hid_node() {
  for device in /sys/class/hidraw/hidraw*/device/uevent; do
    if [ -f "$device" ] && grep -q '^HID_NAME=Keymap Overlay E2E$' "$device"; then
      printf '/dev/%s\n' "$(basename "$(dirname "$(dirname "$device")")")"
      return 0
    fi
  done
  return 1
}

wait_for_virtual_hid_access() {
  node=$1
  attempts=0
  while [ ! -r "$node" ] || [ ! -w "$node" ]; do
    if ! kill -0 "$VIRTUAL_HID_PID" 2>/dev/null; then
      fail 'virtual HID device exited before its udev permissions settled'
    fi
    attempts=$((attempts + 1))
    if [ "$attempts" -ge 100 ]; then
      fail "virtual HID node is not accessible: $node; run make install-uhid-test-rule"
    fi
    sleep 0.05
  done
}

wait_for_state() {
  description=$1
  pattern=$2
  attempts=0
  while [ "$attempts" -lt 100 ]; do
    if ! kill -0 "$DAEMON_PID" 2>/dev/null; then
      fail "daemon exited while waiting for $description"
    fi
    if ! kill -0 "$VIRTUAL_HID_PID" 2>/dev/null; then
      fail "virtual HID device exited while waiting for $description"
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

if [ ! -c /dev/uhid ]; then
  fail '/dev/uhid is unavailable; load the uhid kernel module first'
fi

"$VIRTUAL_HID" --definition "$VIAL_DEFINITION" \
  >"$TEST_DIRECTORY/virtual-hid.log" 2>&1 &
VIRTUAL_HID_PID=$!
attempts=0
while ! grep -q 'Virtual Raw HID device ready' \
  "$TEST_DIRECTORY/virtual-hid.log" 2>/dev/null; do
  if ! kill -0 "$VIRTUAL_HID_PID" 2>/dev/null; then
    fail 'virtual HID device exited during creation'
  fi
  attempts=$((attempts + 1))
  if [ "$attempts" -ge 100 ]; then
    fail 'timed out waiting for virtual HID device creation'
  fi
  sleep 0.05
done

attempts=0
virtual_hid_node=''
while [ -z "$virtual_hid_node" ]; do
  if ! kill -0 "$VIRTUAL_HID_PID" 2>/dev/null; then
    fail 'virtual HID device exited before its hidraw node appeared'
  fi
  attempts=$((attempts + 1))
  if [ "$attempts" -ge 100 ]; then
    fail 'timed out waiting for the virtual HID hidraw node'
  fi
  virtual_hid_node=$(find_virtual_hid_node || true)
  sleep 0.05
done
wait_for_virtual_hid_access "$virtual_hid_node"

"$DAEMON" >"$TEST_DIRECTORY/daemon.log" 2>&1 &
DAEMON_PID=$!

wait_for_state 'the virtual Vial model and lower layer to become visible' \
  ', true, '\''{"version":2,"layer":1'
wait_for_state 'the higher layer to take numeric precedence' \
  ', true, '\''{"version":2,"layer":2'
wait_for_state 'the lower held layer to be restored' \
  ', true, '\''{"version":2,"layer":1'
wait_for_state 'the final release to hide the D-Bus state' ", false, '')"

printf '%s\n' \
  'Linux self-describing Vial HID-to-D-Bus integration test passed'

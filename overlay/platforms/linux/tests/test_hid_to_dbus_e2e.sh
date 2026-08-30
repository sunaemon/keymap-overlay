#!/usr/bin/env sh
set -eu

PROJECT_DIRECTORY=$(CDPATH='' cd -- "$(dirname "$0")/../../../.." && pwd)
DAEMON=${KEYMAP_OVERLAY_E2E_DAEMON:-"$PROJECT_DIRECTORY/target/release/keymap-overlay"}
VIRTUAL_HID=${KEYMAP_OVERLAY_E2E_VIRTUAL_HID:-"$PROJECT_DIRECTORY/target/virtual-raw-hid"}
VIAL_DEFINITION="$PROJECT_DIRECTORY/model/tests/data/vial-contract.json"
UNSUPPORTED_VIAL_DEFINITION="$PROJECT_DIRECTORY/model/tests/data/vial.json"
TEST_DIRECTORY=$(mktemp -d)
DAEMON_PID=''
VIRTUAL_HID_PIDS=''

cleanup() {
  if [ -n "$DAEMON_PID" ]; then
    kill "$DAEMON_PID" 2>/dev/null || true
    wait "$DAEMON_PID" 2>/dev/null || true
  fi
  for virtual_hid_pid in $VIRTUAL_HID_PIDS; do
    kill "$virtual_hid_pid" 2>/dev/null || true
    wait "$virtual_hid_pid" 2>/dev/null || true
  done
  rm -rf "$TEST_DIRECTORY"
}
trap cleanup EXIT HUP INT TERM

fail() {
  printf 'HID-to-D-Bus E2E failure: %s\n' "$1" >&2
  for log_path in "$TEST_DIRECTORY"/*.log; do
    if [ -f "$log_path" ]; then
      printf '%s:\n' "$(basename "$log_path")" >&2
      sed 's/^/  /' "$log_path" >&2
    fi
  done
  exit 1
}

virtual_hids_are_running() {
  for virtual_hid_pid in $VIRTUAL_HID_PIDS; do
    if ! kill -0 "$virtual_hid_pid" 2>/dev/null; then
      return 1
    fi
  done
}

start_virtual_hid() {
  fixture_name=$1
  fixture_mode=$2
  fixture_definition=$3
  fixture_log="$TEST_DIRECTORY/virtual-hid-$fixture_name.log"
  "$VIRTUAL_HID" "$fixture_mode" "$fixture_definition" >"$fixture_log" 2>&1 &
  fixture_pid=$!
  VIRTUAL_HID_PIDS="$VIRTUAL_HID_PIDS $fixture_pid"

  attempts=0
  while ! grep -q 'Virtual Raw HID device ready' "$fixture_log" 2>/dev/null; do
    if ! kill -0 "$fixture_pid" 2>/dev/null; then
      fail "virtual HID fixture $fixture_name exited during creation"
    fi
    attempts=$((attempts + 1))
    if [ "$attempts" -ge 100 ]; then
      fail "timed out waiting for virtual HID fixture $fixture_name"
    fi
    sleep 0.05
  done
}

get_state() {
  gdbus call --session \
    --dest com.sunaemon.KeymapOverlay \
    --object-path /com/sunaemon/KeymapOverlay \
    --method com.sunaemon.KeymapOverlay.Renderer1.GetState 2>/dev/null
}

find_virtual_hid_nodes() {
  found=false
  for device in /sys/class/hidraw/hidraw*/device/uevent; do
    if [ -f "$device" ] && grep -q '^HID_NAME=Keymap Overlay E2E$' "$device"; then
      printf '/dev/%s\n' "$(basename "$(dirname "$(dirname "$device")")")"
      found=true
    fi
  done
  "$found"
}

wait_for_virtual_hid_access() {
  expected=$1
  attempts=0
  while :; do
    nodes=$(find_virtual_hid_nodes || true)
    count=$(printf '%s\n' "$nodes" | sed '/^$/d' | wc -l)
    all_accessible=true
    for node in $nodes; do
      if [ ! -r "$node" ] || [ ! -w "$node" ]; then
        all_accessible=false
      fi
    done
    if [ "$count" -ge "$expected" ] && "$all_accessible"; then
      return
    fi
    if ! virtual_hids_are_running; then
      fail 'a virtual HID fixture exited before its hidraw node became accessible'
    fi
    attempts=$((attempts + 1))
    if [ "$attempts" -ge 100 ]; then
      fail "expected $expected accessible virtual HID nodes, found $count; run make install-uhid-test-rule"
    fi
    sleep 0.05
  done
}

wait_for_state() {
  description=$1
  pattern=$2
  attempts=0
  while [ "$attempts" -lt 200 ]; do
    if ! kill -0 "$DAEMON_PID" 2>/dev/null; then
      fail "daemon exited while waiting for $description"
    fi
    if ! virtual_hids_are_running; then
      fail "a virtual HID fixture exited while waiting for $description"
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

start_virtual_hid unsupported --definition-unsupported "$UNSUPPORTED_VIAL_DEFINITION"
start_virtual_hid invalid --definition-invalid "$VIAL_DEFINITION"
start_virtual_hid handoff-failure --definition-invalid-handoff "$VIAL_DEFINITION"
start_virtual_hid slow --definition-slow "$VIAL_DEFINITION"
wait_for_virtual_hid_access 4

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
  'Linux multi-device Vial HID-to-D-Bus integration test passed'

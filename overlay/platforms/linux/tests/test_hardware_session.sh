#!/bin/bash
# Copyright 2026 sunaemon
# SPDX-License-Identifier: MIT

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../../.." && pwd)"
VIRTUAL_HID="$ROOT/target/virtual-raw-hid"
ACCESSIBILITY_PROBE="$ROOT/target/linux-hil-accessibility"
DRIVER="$ROOT/target/release/keymap-overlay-hil"
VIAL_DEFINITION="$ROOT/model/tests/data/vial-contract.json"
KEYBOARD_ID="${KMO_HIL_KEYBOARD_ID:-1}"
PRIMARY_LAYER="${KMO_HIL_PRIMARY_LAYER:-1}"
LABEL_KEYCODE=0x0068
TRANSCRIPT_DIR="${KMO_HIL_LOG_DIR:-$HOME/.local/var/log/keymap-overlay/hil}"
TRANSCRIPT="$TRANSCRIPT_DIR/linux-session-$(date '+%Y%m%d-%H%M%S').log"
VIRTUAL_HID_PID=""
original_keycode=""
test_row=""
test_column=""
restore_required=false

fail() {
  printf 'ERROR: %s\n' "$*" >&2
  exit 1
}

get_state() {
  gdbus call --session \
    --dest com.sunaemon.KeymapOverlay \
    --object-path /com/sunaemon/KeymapOverlay \
    --method com.sunaemon.KeymapOverlay.Renderer1.GetState 2>/dev/null
}

wait_for_state() {
  local description=$1
  local pattern=$2
  local deadline=$((SECONDS + 15))
  local state=""
  while ((SECONDS < deadline)); do
    state="$(get_state || true)"
    [[ "$state" == *"$pattern"* ]] && return
    sleep 0.05
  done
  fail "Timed out waiting for $description; last state: $state"
}

wait_for_overlay_ready() {
  local deadline=$((SECONDS + 15))
  while ((SECONDS < deadline)); do
    get_state >/dev/null && return
    sleep 0.05
  done
  fail "Timed out waiting for the overlay D-Bus service"
}

wait_for_virtual_hid() {
  local deadline=$((SECONDS + 10))
  while ((SECONDS < deadline)); do
    grep -Fq 'Virtual Raw HID device ready' "$TRANSCRIPT_DIR/virtual-hid.log" \
      2>/dev/null && return
    kill -0 "$VIRTUAL_HID_PID" 2>/dev/null || \
      fail "The virtual HID device exited during startup"
    sleep 0.05
  done
  fail "Timed out waiting for the virtual HID device"
}

journal_cursor() {
  journalctl --user -u keymap-overlay.service -n 0 --show-cursor --no-pager | \
    sed -n 's/^-- cursor: //p'
}

wait_for_event_count() {
  local cursor=$1
  local pattern=$2
  local expected_count=$3
  local deadline=$((SECONDS + 120))
  local events count

  while ((SECONDS < deadline)); do
    events="$(journalctl --user -u keymap-overlay.service \
      --after-cursor "$cursor" --no-pager)"
    count="$(grep -Fc "$pattern" <<<"$events" || true)"
    ((count >= expected_count)) && return
    sleep 0.1
  done

  fail "Expected at least $expected_count '$pattern' events; observed $count"
}

wait_for_accessibility() {
  local expected_label=$1
  local deadline=$((SECONDS + 15))
  local output=""
  local error
  error="$(mktemp "$TRANSCRIPT_DIR/accessibility.err.XXXXXX")"
  while ((SECONDS < deadline)); do
    if output="$($ACCESSIBILITY_PROBE "$expected_label" 2>"$error")"; then
      printf '%s\n' "$output"
      rm -f "$error"
      return
    fi
    sleep 0.1
  done
  cat "$error" >&2
  rm -f "$error"
  fail "Timed out waiting for Qt to publish the overlay accessibility tree"
}

start_overlay() {
  systemctl --user start keymap-overlay.service
  systemctl --user start keymap-overlay-qt.service
}

stop_overlay() {
  systemctl --user stop keymap-overlay-qt.service keymap-overlay.service
}

capture_layer_state() {
  local state
  wait_for_overlay_ready
  "$DRIVER" layer \
    --keyboard-id "$KEYBOARD_ID" --layer "$PRIMARY_LAYER" --state press
  wait_for_state "keyboard $KEYBOARD_ID layer $PRIMARY_LAYER" \
    ", true, '{\"version\":2,\"layer\":$PRIMARY_LAYER"
  state="$(get_state)"
  "$DRIVER" layer \
    --keyboard-id "$KEYBOARD_ID" --layer "$PRIMARY_LAYER" --state release
  wait_for_state "keyboard $KEYBOARD_ID layer $PRIMARY_LAYER release" ", false, '')"
  sed -E 's/^\(uint64 [0-9]+, /(/' <<<"$state"
}

restore_live_keymap() {
  if ! $restore_required; then
    return
  fi
  stop_overlay || return
  if ! "$DRIVER" set-keycode \
    --keyboard-id "$KEYBOARD_ID" --layer 0 --row "$test_row" \
    --column "$test_column" --keycode "$original_keycode"; then
    return 1
  fi
  restore_required=false
}

cleanup() {
  local status=$?
  local restore_failed=false
  set +e
  if ! restore_live_keymap; then
    restore_failed=true
  fi
  stop_overlay
  if [[ -n "$VIRTUAL_HID_PID" ]]; then
    kill "$VIRTUAL_HID_PID" 2>/dev/null
    wait "$VIRTUAL_HID_PID" 2>/dev/null
  fi
  systemctl --user unset-environment QT_LINUX_ACCESSIBILITY_ALWAYS_ON
  if ! start_overlay; then
    restore_failed=true
  fi
  if $restore_failed && ((status == 0)); then
    status=1
  fi
  exit "$status"
}

mkdir -p "$TRANSCRIPT_DIR" "$ROOT/target"
exec > >(tee "$TRANSCRIPT") 2>&1
trap cleanup EXIT

[[ "$(uname -s)" == Linux ]] || fail "This test requires Linux"
[[ "$XDG_SESSION_TYPE" == wayland ]] || fail "This test requires Wayland"
[[ "$XDG_CURRENT_DESKTOP" == *KDE* ]] || \
  fail "The accessibility integration currently requires KDE Plasma"
[[ -z "$(git -C "$ROOT" status --short)" ]] || \
  fail "Candidate worktree is not clean"
[[ -r /dev/uhid && -w /dev/uhid ]] || \
  fail "/dev/uhid must be readable and writable; load uhid and grant a user ACL"

printf 'Candidate: %s\n' "$(git -C "$ROOT" rev-parse HEAD)"
printf 'Desktop: %s\nSession: %s\n' "$XDG_CURRENT_DESKTOP" "$XDG_SESSION_TYPE"

make -C "$ROOT" install-overlay
make -C "$ROOT" build-hil-driver-linux
"${CC:-cc}" -o "$VIRTUAL_HID" -std=c11 -Wall -Wextra -Wpedantic -Werror \
  "$ROOT/overlay/platforms/linux/tests/virtual_raw_hid.c" -llzma
read -r -a atspi_flags <<<"$(pkg-config --cflags --libs atspi-2 gobject-2.0)"
"${CC:-cc}" -o "$ACCESSIBILITY_PROBE" \
  -std=c11 -Wall -Wextra -Wpedantic -Werror \
  "$ROOT/overlay/platforms/linux/tests/hil_accessibility.c" \
  "${atspi_flags[@]}"

"$DRIVER" devices
"$DRIVER" probe --keyboard-id "$KEYBOARD_ID"
coordinates="$(
  "$DRIVER" find-transparent \
    --keyboard-id "$KEYBOARD_ID" --layer "$PRIMARY_LAYER"
)"
read -r row_field column_field original_field <<<"$coordinates"
test_row="${row_field#row=}"
test_column="${column_field#column=}"
original_keycode="${original_field#original=}"
[[ -n "$test_row" && -n "$test_column" && -n "$original_keycode" ]] || \
  fail "Could not select a transparent Vial test position"

baseline_state="$(capture_layer_state)"
stop_overlay
"$DRIVER" set-keycode \
  --keyboard-id "$KEYBOARD_ID" --layer 0 --row "$test_row" \
  --column "$test_column" --keycode "$LABEL_KEYCODE"
restore_required=true
start_overlay
edited_state="$(capture_layer_state)"
[[ "$edited_state" == *F13* ]] || fail "The restarted daemon did not load the F13 edit"
[[ "$edited_state" != "$baseline_state" ]] || fail "The Vial edit did not change the model"

restore_live_keymap
start_overlay
restored_state="$(capture_layer_state)"
[[ "$restored_state" == "$baseline_state" ]] || \
  fail "The restored Vial keymap did not reproduce the original model"
printf '%s\n' 'PASS: live Vial keycode edit appeared only after restart and restored exactly'

stop_overlay
systemctl --user set-environment QT_LINUX_ACCESSIBILITY_ALWAYS_ON=1
cursor="$(journal_cursor)"
"$VIRTUAL_HID" --definition "$VIAL_DEFINITION" \
  >"$TRANSCRIPT_DIR/virtual-hid.log" 2>&1 &
VIRTUAL_HID_PID=$!
wait_for_virtual_hid
start_overlay

focused_before="$($ACCESSIBILITY_PROBE -)"
wait_for_state "the lower virtual layer" \
  ", true, '{\"version\":2,\"layer\":1"
accessibility_result="$(wait_for_accessibility L1)"
printf '%s\n' "$accessibility_result"
focused_during="FOCUSED=${accessibility_result#*focused=}"
focused_during="${focused_during% overlay-focused=false}"
[[ "$focused_before" == "$focused_during" ]] || \
  fail "The overlay changed accessibility focus: before=$focused_before during=$focused_during"

wait_for_state "numeric precedence for the higher virtual layer" \
  ", true, '{\"version\":2,\"layer\":2"
wait_for_state "restoration of the lower virtual layer" \
  ", true, '{\"version\":2,\"layer\":1"
wait_for_state "the final virtual release" ", false, '')"
wait_for_event_count "$cursor" \
  'Layer event: keyboard=7 layer=1 pressed=true' 10
wait_for_event_count "$cursor" \
  'Layer event: keyboard=7 layer=1 pressed=false' 10
wait_for_state "the tenth final virtual release" ", false, '')"

systemctl --user --quiet is-active keymap-overlay.service
systemctl --user --quiet is-active keymap-overlay-qt.service
journalctl --user -u keymap-overlay.service --since '2 minutes ago' --no-pager

printf '%s\n' \
  'PASS: Linux live Vial restart read, installed virtual Vial device, ten Raw HID cycles, ordering, D-Bus state, Qt accessibility labels, and focus retention'
printf 'Transcript: %s\n' "$TRANSCRIPT"

#!/bin/bash
# Copyright 2026 sunaemon
# SPDX-License-Identifier: MIT

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../../.." && pwd)"
DRIVER="$ROOT/target/release/keymap-overlay-hil"
UI_PROBE_APP="$ROOT/target/hil/KeymapOverlayHIL.app"
KEYBOARD_ID="${KMO_HIL_KEYBOARD_ID:-1}"
SECONDARY_KEYBOARD_ID="${KMO_HIL_SECONDARY_KEYBOARD_ID:-2}"
PRIMARY_LAYER="${KMO_HIL_PRIMARY_LAYER:-1}"
SECONDARY_LAYER="${KMO_HIL_SECONDARY_LAYER:-2}"
ENCODER_KEYBOARD_ID="${KMO_HIL_ENCODER_KEYBOARD_ID:-2}"
ENCODER_LAYER="${KMO_HIL_ENCODER_LAYER:-1}"
LABEL_KEYCODE=0x0068
LABEL=F13
ENCODER_TEST_KEYCODES=(0x0004 0x0005 0x0006 0x0007 0x0008 0x0009)
ENCODER_LABELS=A,B,C,D,E,F
ENCODER_MAC_KEYCODES=0,11,8,2,14,3
SERVICE_LABEL=com.sunaemon.keymap-overlay
PLIST="$HOME/Library/LaunchAgents/$SERVICE_LABEL.plist"
LOG="$HOME/.local/var/log/keymap-overlay/overlay.log"
TRANSCRIPT_DIR="${KMO_HIL_LOG_DIR:-$HOME/.local/var/log/keymap-overlay/hil}"
TRANSCRIPT="$TRANSCRIPT_DIR/macos-session-$(date '+%Y%m%d-%H%M%S').log"

original_keycode=""
test_row=""
test_column=""
restore_required=false
encoder_restore_required=false
encoder_original_keycodes=()

fail() {
  printf 'ERROR: %s\n' "$*" >&2
  exit 1
}

stop_overlay() {
  local output
  if output="$(launchctl bootout "gui/$(id -u)/$SERVICE_LABEL" 2>&1)"; then
    return
  fi
  case "$output" in
    ""|*"Could not find service"*|*"No such process"*) ;;
    *) printf '%s\n' "$output" >&2; return 1 ;;
  esac
}

run_ui_probe() {
  local expected=$1
  local output error
  shift
  output="$(mktemp "$TRANSCRIPT_DIR/macos-ui-probe.out.XXXXXX")"
  error="$(mktemp "$TRANSCRIPT_DIR/macos-ui-probe.err.XXXXXX")"
  if ! open -W -n -o "$output" --stderr "$error" "$UI_PROBE_APP" --args "$@"; then
    cat "$output" "$error"
    fail "LaunchServices could not run the HIL UI app"
  fi
  cat "$output" "$error"
  grep -Fqx "$expected" "$output" || \
    fail "The HIL UI app did not report a successful result"
}

restore_live_keymap() {
  local status=$?
  set +e
  "$DRIVER" layer --keyboard-id "$KEYBOARD_ID" --layer "$PRIMARY_LAYER" --state release
  "$DRIVER" layer --keyboard-id "$KEYBOARD_ID" --layer "$SECONDARY_LAYER" --state release
  "$DRIVER" layer \
    --keyboard-id "$SECONDARY_KEYBOARD_ID" --layer "$PRIMARY_LAYER" --state release
  if $restore_required; then
    stop_overlay
    "$DRIVER" set-keycode \
      --keyboard-id "$KEYBOARD_ID" --layer 0 --row "$test_row" \
      --column "$test_column" --keycode "$original_keycode"
  fi
  if $encoder_restore_required; then
    local binding=0
    local index direction
    stop_overlay
    for index in 0 1 2; do
      for direction in ccw cw; do
        "$DRIVER" set-encoder \
          --keyboard-id "$ENCODER_KEYBOARD_ID" --layer 0 --index "$index" \
          --direction "$direction" --keycode "${encoder_original_keycodes[$binding]}"
        binding=$((binding + 1))
      done
    done
  fi
  if $restore_required || $encoder_restore_required; then
    make -C "$ROOT" install-overlay
  fi
  exit "$status"
}

mkdir -p "$TRANSCRIPT_DIR" "$ROOT/target/hil"
exec > >(tee "$TRANSCRIPT") 2>&1
trap restore_live_keymap EXIT

[[ "$(uname -s)" == Darwin ]] || fail "This test requires macOS"
[[ "$(uname -m)" == arm64 ]] || fail "This release row requires macOS arm64"
[[ -z "$(git -C "$ROOT" status --short)" ]] || fail "Candidate worktree is not clean"

printf 'Candidate: %s\n' "$(git -C "$ROOT" rev-parse HEAD)"
make -C "$ROOT" build-hil-macos
run_ui_probe "Accessibility permission is available" --check-accessibility

"$DRIVER" devices
"$DRIVER" probe --keyboard-id "$KEYBOARD_ID"
"$DRIVER" probe --keyboard-id "$SECONDARY_KEYBOARD_ID"

coordinates="$($DRIVER find-transparent --keyboard-id "$KEYBOARD_ID" --layer "$PRIMARY_LAYER")"
read -r row_field column_field original_field <<<"$coordinates"
test_row="${row_field#row=}"
test_column="${column_field#column=}"
original_keycode="${original_field#original=}"
[[ -n "$test_row" && -n "$test_column" && -n "$original_keycode" ]] || \
  fail "Could not select a transparent Vial test position"

stop_overlay
"$DRIVER" set-keycode \
  --keyboard-id "$KEYBOARD_ID" --layer 0 --row "$test_row" \
  --column "$test_column" --keycode "$LABEL_KEYCODE"
restore_required=true

binding=0
for index in 0 1 2; do
  for direction in ccw cw; do
    encoder_original_keycodes+=("$(
      "$DRIVER" get-encoder \
        --keyboard-id "$ENCODER_KEYBOARD_ID" --layer 0 --index "$index" \
        --direction "$direction"
    )")
    binding=$((binding + 1))
  done
done
encoder_restore_required=true
binding=0
for index in 0 1 2; do
  for direction in ccw cw; do
    "$DRIVER" set-encoder \
      --keyboard-id "$ENCODER_KEYBOARD_ID" --layer 0 --index "$index" \
      --direction "$direction" --keycode "${ENCODER_TEST_KEYCODES[$binding]}"
    binding=$((binding + 1))
  done
done

log_start=1
if [[ -f "$LOG" ]]; then
  log_start="$(( $(wc -l <"$LOG") + 1 ))"
fi
make -C "$ROOT" install-overlay

[[ -f "$PLIST" ]] || fail "The macOS LaunchAgent plist was not installed"
if grep -Eq -- '--asset-dir|--keyboard-config-dir' "$PLIST"; then
  fail "The installed LaunchAgent still contains an obsolete model argument"
fi

service_state="$(launchctl print "gui/$(id -u)/$SERVICE_LABEL")"
overlay_pid="$(awk '$1 == "pid" && $2 == "=" { print $3; exit }' <<<"$service_state")"
[[ "$overlay_pid" =~ ^[0-9]+$ ]] || fail "The installed overlay has no running PID"

sleep 1
new_logs="$(tail -n "+$log_start" "$LOG")"
if grep -Eqi 'failed to (open|read)|Vial.*error|model.*error|Raw HID.*error' <<<"$new_logs"; then
  printf '%s\n' "$new_logs" >&2
  fail "The current overlay start logged a device/model error"
fi

run_ui_probe "macOS Accessibility HIL checks passed" \
  --overlay-pid "$overlay_pid" \
  --driver "$DRIVER" \
  --keyboard-id "$KEYBOARD_ID" \
  --secondary-keyboard-id "$SECONDARY_KEYBOARD_ID" \
  --layer "$PRIMARY_LAYER" \
  --secondary-layer "$SECONDARY_LAYER" \
  --expected-label "$LABEL" \
  --encoder-keyboard-id "$ENCODER_KEYBOARD_ID" \
  --encoder-layer "$ENCODER_LAYER" \
  --encoder-labels "$ENCODER_LABELS" \
  --encoder-key-codes "$ENCODER_MAC_KEYCODES"

printf 'PASS: macOS live startup, Vial reread, labels, layer transitions, focus, '\
'click-through, topmost, attached-display placement, simultaneous keyboards, '\
'and synthetic encoder rotations through live firmware output\n'
printf 'Transcript: %s\n' "$TRANSCRIPT"

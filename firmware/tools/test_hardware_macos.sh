#!/bin/bash
# Copyright 2026 sunaemon
# SPDX-License-Identifier: GPL-2.0-or-later

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
DRIVER="$ROOT/target/release/keymap-overlay-hil"
BOOTLOADER_CONTROL="${KMO_HIL_BOOTLOADER_CONTROL:-}"
USB_CONTROL="${KMO_HIL_USB_CONTROL:-}"
KEYBOARD_IDS_TEXT="${KMO_HIL_KEYBOARD_IDS:-1 2}"
PRIMARY_LAYER="${KMO_HIL_PRIMARY_LAYER:-1}"
LABEL_KEYCODE=0x0068
LOG="$HOME/.local/var/log/keymap-overlay/overlay.log"
TRANSCRIPT_DIR="${KMO_HIL_LOG_DIR:-$HOME/.local/var/log/keymap-overlay/hil}"
TRANSCRIPT="$TRANSCRIPT_DIR/macos-firmware-$(date '+%Y%m%d-%H%M%S').log"

read -r -a keyboard_ids <<<"$KEYBOARD_IDS_TEXT"
restore_ids=()
restore_rows=()
restore_columns=()
restore_keycodes=()
cleanup_enabled=false

fail() {
  printf 'ERROR: %s\n' "$*" >&2
  exit 1
}

wait_for_keyboard() {
  local keyboard_id=$1
  local deadline=$((SECONDS + 30))
  while (( SECONDS < deadline )); do
    if "$DRIVER" probe --keyboard-id "$keyboard_id" >/dev/null 2>&1; then
      return
    fi
    sleep 0.5
  done
  fail "Keyboard $keyboard_id did not return with HIL firmware"
}

wait_for_new_log() {
  local start_line=$1
  local pattern=$2
  local deadline=$((SECONDS + 10))
  while (( SECONDS < deadline )); do
    if [[ -f "$LOG" ]] && tail -n "+$start_line" "$LOG" | grep -Fq "$pattern"; then
      return
    fi
    sleep 0.1
  done
  fail "Timed out waiting for new log entry: $pattern"
}

flash_keyboard() {
  local keyboard_id=$1
  "$BOOTLOADER_CONTROL" enter "$keyboard_id"
  make -C "$ROOT" flash KEYBOARD_ID="$keyboard_id"
  wait_for_keyboard "$keyboard_id"
}

power_cycle_keyboard() {
  local keyboard_id=$1
  "$USB_CONTROL" off "$keyboard_id"
  sleep 1
  "$USB_CONTROL" on "$keyboard_id"
  wait_for_keyboard "$keyboard_id"
}

cleanup() {
  local status=$?
  set +e
  if ! $cleanup_enabled; then
    exit "$status"
  fi
  if [[ -x "$USB_CONTROL" ]]; then
    for keyboard_id in "${keyboard_ids[@]}"; do
      "$USB_CONTROL" on "$keyboard_id"
    done
  fi
  for index in "${!restore_ids[@]}"; do
    if "$DRIVER" probe --keyboard-id "${restore_ids[$index]}" >/dev/null 2>&1; then
      "$DRIVER" set-keycode \
        --keyboard-id "${restore_ids[$index]}" --layer 0 \
        --row "${restore_rows[$index]}" --column "${restore_columns[$index]}" \
        --keycode "${restore_keycodes[$index]}"
    fi
  done
  if [[ -x "$DRIVER" ]]; then
    make -C "$ROOT" install-overlay
  fi
  exit "$status"
}

mkdir -p "$TRANSCRIPT_DIR"
exec > >(tee "$TRANSCRIPT") 2>&1
trap cleanup EXIT

[[ "$(uname -s)" == Darwin ]] || fail "This test requires macOS"
[[ "$(uname -m)" == arm64 ]] || fail "This release row requires macOS arm64"
[[ -z "$(git -C "$ROOT" status --short)" ]] || fail "Candidate worktree is not clean"
[[ -x "$BOOTLOADER_CONTROL" ]] || \
  fail "Set KMO_HIL_BOOTLOADER_CONTROL to the executable that enters each board's bootloader"
[[ -x "$USB_CONTROL" ]] || \
  fail "Set KMO_HIL_USB_CONTROL to the executable controlling each keyboard's USB port"
(( ${#keyboard_ids[@]} > 0 )) || fail "KMO_HIL_KEYBOARD_IDS is empty"

printf 'Candidate: %s\n' "$(git -C "$ROOT" rev-parse HEAD)"
make -C "$ROOT" build-hil-macos
cleanup_enabled=true

for keyboard_id in "${keyboard_ids[@]}"; do
  printf 'Flashing keyboard %s with the candidate HIL firmware\n' "$keyboard_id"
  flash_keyboard "$keyboard_id"
  "$DRIVER" reset-keymap --keyboard-id "$keyboard_id"

  coordinates="$($DRIVER find-transparent --keyboard-id "$keyboard_id" --layer "$PRIMARY_LAYER")"
  read -r row_field column_field default_field <<<"$coordinates"
  row="${row_field#row=}"
  column="${column_field#column=}"
  compiled_default="${default_field#original=}"
  restore_ids+=("$keyboard_id")
  restore_rows+=("$row")
  restore_columns+=("$column")
  restore_keycodes+=("$compiled_default")

  "$DRIVER" set-keycode \
    --keyboard-id "$keyboard_id" --layer 0 --row "$row" --column "$column" \
    --keycode "$LABEL_KEYCODE"
  [[ "$($DRIVER get-keycode --keyboard-id "$keyboard_id" --layer 0 --row "$row" --column "$column")" == "$LABEL_KEYCODE" ]] || \
    fail "Keyboard $keyboard_id did not accept the pre-flash Vial edit"

  flash_keyboard "$keyboard_id"
  [[ "$($DRIVER get-keycode --keyboard-id "$keyboard_id" --layer 0 --row "$row" --column "$column")" == "$compiled_default" ]] || \
    fail "Keyboard $keyboard_id did not restore compiled defaults after flashing"

  "$DRIVER" set-keycode \
    --keyboard-id "$keyboard_id" --layer 0 --row "$row" --column "$column" \
    --keycode "$LABEL_KEYCODE"
  power_cycle_keyboard "$keyboard_id"
  [[ "$($DRIVER get-keycode --keyboard-id "$keyboard_id" --layer 0 --row "$row" --column "$column")" == "$LABEL_KEYCODE" ]] || \
    fail "Keyboard $keyboard_id lost a Vial edit across reconnect"
  "$DRIVER" set-keycode \
    --keyboard-id "$keyboard_id" --layer 0 --row "$row" --column "$column" \
    --keycode "$compiled_default"
  printf 'PASS: keyboard %s flash, compiled defaults, and EEPROM persistence\n' "$keyboard_id"
done

disconnect_id="${keyboard_ids[0]}"
make -C "$ROOT" install-overlay
log_start="$(( $(wc -l <"$LOG") + 1 ))"
"$DRIVER" layer --keyboard-id "$disconnect_id" --layer "$PRIMARY_LAYER" --state press
wait_for_new_log "$log_start" "show keyboard=$disconnect_id layers=[$PRIMARY_LAYER]"
"$USB_CONTROL" off "$disconnect_id"
wait_for_new_log "$log_start" "hide size=1x1"
"$USB_CONTROL" on "$disconnect_id"
wait_for_keyboard "$disconnect_id"
"$DRIVER" layer --keyboard-id "$disconnect_id" --layer "$PRIMARY_LAYER" --state press
wait_for_new_log "$log_start" "Layer event: keyboard=$disconnect_id layer=$PRIMARY_LAYER pressed=true"
"$DRIVER" layer --keyboard-id "$disconnect_id" --layer "$PRIMARY_LAYER" --state release

"$USB_CONTROL" off "$disconnect_id"
make -C "$ROOT" install-overlay
log_start="$(( $(wc -l <"$LOG") + 1 ))"
"$USB_CONTROL" on "$disconnect_id"
wait_for_keyboard "$disconnect_id"
"$DRIVER" layer --keyboard-id "$disconnect_id" --layer "$PRIMARY_LAYER" --state press
wait_for_new_log "$log_start" \
  "Overlay model is unavailable for keyboard $disconnect_id, layers [$PRIMARY_LAYER]"
"$DRIVER" layer --keyboard-id "$disconnect_id" --layer "$PRIMARY_LAYER" --state release
make -C "$ROOT" install-overlay
log_start="$(( $(wc -l <"$LOG") + 1 ))"
"$DRIVER" layer --keyboard-id "$disconnect_id" --layer "$PRIMARY_LAYER" --state press
wait_for_new_log "$log_start" "show keyboard=$disconnect_id layers=[$PRIMARY_LAYER]"
"$DRIVER" layer --keyboard-id "$disconnect_id" --layer "$PRIMARY_LAYER" --state release

printf 'PASS: disconnect, reconnect, absent-at-startup, and restart behavior\n'
printf 'Transcript: %s\n' "$TRANSCRIPT"

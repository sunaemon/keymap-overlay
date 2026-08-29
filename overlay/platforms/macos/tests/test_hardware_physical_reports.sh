#!/bin/bash
# Copyright 2026 sunaemon
# SPDX-License-Identifier: MIT

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../../.." && pwd)"
DRIVER="$ROOT/target/release/keymap-overlay-hil"
SERVICE_LABEL=com.sunaemon.keymap-overlay
PLIST="$HOME/Library/LaunchAgents/$SERVICE_LABEL.plist"
EXPECTED_REPORTS_TEXT="${KMO_HIL_PHYSICAL_REPORTS:-1:1 1:2 2:3}"
TIMEOUT_SECONDS="${KMO_HIL_PHYSICAL_TIMEOUT_SECONDS:-90}"
TRANSCRIPT_DIR="${KMO_HIL_LOG_DIR:-$HOME/.local/var/log/keymap-overlay/hil}"
TRANSCRIPT="$TRANSCRIPT_DIR/macos-physical-reports-$(date '+%Y%m%d-%H%M%S').log"

fail() {
  printf 'ERROR: %s\n' "$*" >&2
  exit 1
}

physical_key_label() {
  case "$1:$2" in
    1:1) printf 'Insixty far-right key on the Z row (MO(1))' ;;
    1:2) printf 'Insixty bottom-left key (MO(2))' ;;
    2:3) printf 'DOIO bottom-left key (MO(3))' ;;
    *) printf 'keyboard ID %s MO(%s) key' "$1" "$2" ;;
  esac
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

start_overlay() {
  launchctl bootstrap "gui/$(id -u)" "$PLIST"
}

observe_physical_tap() {
  local keyboard_id=$1
  local layer=$2
  local label
  label="$(physical_key_label "$keyboard_id" "$layer")"

  printf '\nACTION: Quickly tap %s once.\n' "$label"
  "$DRIVER" observe-layer \
    --keyboard-id "$keyboard_id" \
    --layer "$layer" \
    --timeout-ms "$((TIMEOUT_SECONDS * 1000))"
  printf 'PASS: keyboard=%s layer=%s physical press/release report\n' \
    "$keyboard_id" "$layer"
}

restore_overlay() {
  local status=$?
  set +e
  if $overlay_stopped; then
    start_overlay
  fi
  exit "$status"
}

mkdir -p "$TRANSCRIPT_DIR"
exec > >(tee "$TRANSCRIPT") 2>&1
overlay_stopped=false
trap restore_overlay EXIT

[[ "$(uname -s)" == Darwin ]] || fail "This test requires macOS"
[[ "$(uname -m)" == arm64 ]] || fail "This release row requires macOS arm64"
[[ -z "$(git -C "$ROOT" status --short)" ]] || fail "Candidate worktree is not clean"
[[ "$TIMEOUT_SECONDS" =~ ^[1-9][0-9]*$ ]] || \
  fail "KMO_HIL_PHYSICAL_TIMEOUT_SECONDS must be positive"

read -r -a expected_reports <<<"$EXPECTED_REPORTS_TEXT"
(( ${#expected_reports[@]} > 0 )) || fail "KMO_HIL_PHYSICAL_REPORTS is empty"
for report in "${expected_reports[@]}"; do
  [[ "$report" =~ ^[0-9]+:[1-9][0-9]*$ ]] || \
    fail "Invalid physical report '$report'; expected KEYBOARD_ID:LAYER"
done

printf 'Candidate: %s\n' "$(git -C "$ROOT" rev-parse HEAD)"
printf 'Physical reports: %s\n' "$EXPECTED_REPORTS_TEXT"

make -C "$ROOT" build-hil-driver-macos
make -C "$ROOT" install-overlay
[[ -f "$PLIST" ]] || fail "The macOS LaunchAgent plist was not installed"
stop_overlay
overlay_stopped=true

for report in "${expected_reports[@]}"; do
  observe_physical_tap "${report%%:*}" "${report#*:}"
done

start_overlay
overlay_stopped=false

printf '\nPASS: every configured physical MO key emitted ordered press/release Raw HID reports\n'
printf 'No deterministic HIL layer command was used by this test.\n'
printf 'Transcript: %s\n' "$TRANSCRIPT"

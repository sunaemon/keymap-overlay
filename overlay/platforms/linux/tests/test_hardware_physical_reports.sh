#!/bin/bash
# Copyright 2026 sunaemon
# SPDX-License-Identifier: MIT

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../../.." && pwd)"
EXPECTED_REPORTS_TEXT="${KMO_HIL_PHYSICAL_REPORTS:-1:1 1:2 2:3}"
KEYBOARD_COUNT="${KMO_HIL_KEYBOARD_COUNT:-2}"
TIMEOUT_SECONDS="${KMO_HIL_PHYSICAL_TIMEOUT_SECONDS:-180}"
TRANSCRIPT_DIR="${KMO_HIL_LOG_DIR:-$HOME/.local/var/log/keymap-overlay/hil}"
TRANSCRIPT="$TRANSCRIPT_DIR/linux-physical-reports-$(date '+%Y%m%d-%H%M%S').log"

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

journal_cursor() {
  journalctl --user -u keymap-overlay.service -n 0 --show-cursor --no-pager | \
    sed -n 's/^-- cursor: //p'
}

wait_for_startup_devices() {
  local deadline
  deadline=$((SECONDS + TIMEOUT_SECONDS))

  while ((SECONDS < deadline)); do
    if journalctl --user -u keymap-overlay.service --since '1 minute ago' \
      --no-pager | grep -F "Adopted $KEYBOARD_COUNT startup Raw HID device(s)" \
      >/dev/null; then
      return
    fi
    sleep 0.1
  done

  fail "The daemon did not adopt $KEYBOARD_COUNT startup devices"
}

observe_physical_tap() {
  local keyboard_id=$1
  local layer=$2
  local label cursor pattern deadline events states state_count
  label="$(physical_key_label "$keyboard_id" "$layer")"
  cursor="$(journal_cursor)"
  [[ -n "$cursor" ]] || fail "Could not capture the overlay journal cursor"
  pattern="Layer event: keyboard=$keyboard_id layer=$layer pressed="
  deadline=$((SECONDS + TIMEOUT_SECONDS))

  printf '\nACTION: Quickly tap %s once.\n' "$label"
  printf 'Waiting for physical press and release reports (timeout: %ss)...\n' \
    "$TIMEOUT_SECONDS"

  states=""
  events=""
  while ((SECONDS < deadline)); do
    events="$(journalctl --user -u keymap-overlay.service \
      --after-cursor "$cursor" --no-pager | grep -F "$pattern" || true)"
    states="$(printf '%s\n' "$events" | sed -En \
      "s/.*Layer event: keyboard=$keyboard_id layer=$layer pressed=(true|false).*/\1/p")"
    state_count="$(printf '%s\n' "$states" | sed '/^$/d' | wc -l | tr -d ' ')"
    ((state_count >= 2)) && break
    sleep 0.1
  done

  [[ "$states" == $'true\nfalse' ]] || {
    [[ -z "$events" ]] || printf '%s\n' "$events" >&2
    fail "Expected exactly one ordered physical press/release for keyboard=$keyboard_id layer=$layer"
  }
  printf '%s\n' "$events"
  printf 'PASS: keyboard=%s layer=%s physical press/release report\n' \
    "$keyboard_id" "$layer"
}

mkdir -p "$TRANSCRIPT_DIR"
exec > >(tee "$TRANSCRIPT") 2>&1

[[ "$(uname -s)" == Linux ]] || fail "This test requires Linux"
[[ -z "$(git -C "$ROOT" status --short)" ]] || fail "Candidate worktree is not clean"
[[ "$KEYBOARD_COUNT" =~ ^[1-9][0-9]*$ ]] || fail "KMO_HIL_KEYBOARD_COUNT must be positive"
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
make -C "$ROOT" install-overlay
systemctl --user --quiet is-active keymap-overlay.service || \
  fail "The installed overlay daemon is not active"
wait_for_startup_devices

for report in "${expected_reports[@]}"; do
  observe_physical_tap "${report%%:*}" "${report#*:}"
done

printf '\nPASS: every configured physical MO key emitted ordered press/release Raw HID reports\n'
printf 'Transcript: %s\n' "$TRANSCRIPT"

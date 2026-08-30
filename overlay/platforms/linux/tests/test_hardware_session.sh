#!/bin/bash
# Copyright 2026 sunaemon
# SPDX-License-Identifier: MIT

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../../.." && pwd)"
VIRTUAL_HID="$ROOT/target/virtual-raw-hid"
ACCESSIBILITY_PROBE="$ROOT/target/linux-hil-accessibility"
VIAL_DEFINITION="$ROOT/model/tests/data/vial-contract.json"
TRANSCRIPT_DIR="${KMO_HIL_LOG_DIR:-$HOME/.local/var/log/keymap-overlay/hil}"
TRANSCRIPT="$TRANSCRIPT_DIR/linux-session-$(date '+%Y%m%d-%H%M%S').log"
VIRTUAL_HID_PID=""

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

cleanup() {
  local status=$?
  set +e
  systemctl --user stop keymap-overlay-qt.service keymap-overlay.service
  if [[ -n "$VIRTUAL_HID_PID" ]]; then
    kill "$VIRTUAL_HID_PID" 2>/dev/null
    wait "$VIRTUAL_HID_PID" 2>/dev/null
  fi
  systemctl --user unset-environment QT_LINUX_ACCESSIBILITY_ALWAYS_ON
  systemctl --user start keymap-overlay.service keymap-overlay-qt.service
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
"${CC:-cc}" -o "$VIRTUAL_HID" -std=c11 -Wall -Wextra -Wpedantic -Werror \
  "$ROOT/overlay/platforms/linux/tests/virtual_raw_hid.c" -llzma
read -r -a atspi_flags <<<"$(pkg-config --cflags --libs atspi-2 gobject-2.0)"
"${CC:-cc}" -o "$ACCESSIBILITY_PROBE" \
  -std=c11 -Wall -Wextra -Wpedantic -Werror \
  "$ROOT/overlay/platforms/linux/tests/hil_accessibility.c" \
  "${atspi_flags[@]}"

systemctl --user stop keymap-overlay-qt.service keymap-overlay.service
systemctl --user set-environment QT_LINUX_ACCESSIBILITY_ALWAYS_ON=1
cursor="$(journal_cursor)"
"$VIRTUAL_HID" --definition "$VIAL_DEFINITION" \
  >"$TRANSCRIPT_DIR/virtual-hid.log" 2>&1 &
VIRTUAL_HID_PID=$!
wait_for_virtual_hid
systemctl --user start keymap-overlay.service keymap-overlay-qt.service

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
  'PASS: installed Linux virtual Vial device, ten Raw HID cycles, ordering, D-Bus state, Qt accessibility labels, and focus retention'
printf 'Transcript: %s\n' "$TRANSCRIPT"

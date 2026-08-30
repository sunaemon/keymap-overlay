#!/bin/bash
# Copyright 2026 sunaemon
# SPDX-License-Identifier: MIT

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../../.." && pwd)"
LABEL=com.sunaemon.keymap-overlay.hil-login
OVERLAY_LABEL=com.sunaemon.keymap-overlay
PLIST="$HOME/Library/LaunchAgents/$LABEL.plist"
TEMPLATE="$ROOT/overlay/platforms/macos/tests/login_hil.plist"
DRIVER="$ROOT/target/release/keymap-overlay-hil"
UI_PROBE_APP="$ROOT/target/hil/KeymapOverlayHIL.app"
UI_PROBE="$UI_PROBE_APP/Contents/MacOS/keymap-overlay-macos-hil-ui"
HIL_LOG_DIR="${KMO_HIL_LOG_DIR:-$HOME/.local/var/log/keymap-overlay/hil}"
RESULT="$HIL_LOG_DIR/macos-login-result.txt"
STDOUT_LOG="$HIL_LOG_DIR/macos-login.out.log"
STDERR_LOG="$HIL_LOG_DIR/macos-login.err.log"
KEYBOARD_ID="${KMO_HIL_KEYBOARD_ID:-1}"
SECONDARY_KEYBOARD_ID="${KMO_HIL_SECONDARY_KEYBOARD_ID:-2}"
PRIMARY_LAYER="${KMO_HIL_PRIMARY_LAYER:-1}"
SECONDARY_LAYER="${KMO_HIL_SECONDARY_LAYER:-2}"

fail() {
  printf 'ERROR: %s\n' "$*" >&2
  exit 1
}

run_ui_probe() {
  local expected=$1
  local output error
  shift
  output="$(mktemp "$HIL_LOG_DIR/macos-login-ui.out.XXXXXX")"
  error="$(mktemp "$HIL_LOG_DIR/macos-login-ui.err.XXXXXX")"
  if ! "$UI_PROBE" "$@" >"$output" 2>"$error"; then
    cat "$output" "$error"
    fail "The HIL UI app failed"
  fi
  cat "$output" "$error"
  grep -Fqx "$expected" "$output" || \
    fail "The HIL UI app did not report a successful result"
}

prepare() {
  local request_logout=${1:-}
  [[ "$(uname -s)" == Darwin ]] || fail "This test requires macOS"
  [[ "$(uname -m)" == arm64 ]] || fail "This release row requires macOS arm64"
  [[ -z "$(git -C "$ROOT" status --short)" ]] || fail "Candidate worktree is not clean"

  mkdir -p "$HIL_LOG_DIR" "$ROOT/target/hil" "$HOME/Library/LaunchAgents"
  make -C "$ROOT" build-hil-macos
  run_ui_probe "Accessibility permission is available" --check-accessibility
  "$DRIVER" probe --keyboard-id "$KEYBOARD_ID"
  "$DRIVER" probe --keyboard-id "$SECONDARY_KEYBOARD_ID"
  make -C "$ROOT" install-overlay

  rm -f "$RESULT" "$STDOUT_LOG" "$STDERR_LOG"
  cp "$TEMPLATE" "$PLIST"
  plutil -insert ProgramArguments.0 -string "$ROOT/overlay/platforms/macos/tests/test_hardware_login.sh" "$PLIST"
  plutil -insert ProgramArguments.1 -string after-login "$PLIST"
  plutil -insert ProgramArguments.2 -string "$(git -C "$ROOT" rev-parse HEAD)" "$PLIST"
  plutil -insert ProgramArguments.3 -string "$KEYBOARD_ID" "$PLIST"
  plutil -insert ProgramArguments.4 -string "$SECONDARY_KEYBOARD_ID" "$PLIST"
  plutil -insert ProgramArguments.5 -string "$PRIMARY_LAYER" "$PLIST"
  plutil -insert ProgramArguments.6 -string "$SECONDARY_LAYER" "$PLIST"
  plutil -replace StandardOutPath -string "$STDOUT_LOG" "$PLIST"
  plutil -replace StandardErrorPath -string "$STDERR_LOG" "$PLIST"
  plutil -lint "$PLIST"

  printf 'Login continuation prepared for candidate %s.\n' "$(git -C "$ROOT" rev-parse HEAD)"
  printf 'The next sign-in will record service and keyboard readiness.\n'
  if [[ "$request_logout" == --logout ]]; then
    osascript -e 'tell application "System Events" to log out'
  else
    printf 'Sign out and sign in, then run: make verify-hardware-login-macos\n'
  fi
}

after_login() {
  local candidate=$1
  local keyboard_id=$2
  local secondary_keyboard_id=$3
  local primary_layer=$4
  local secondary_layer=$5
  local deadline=$((SECONDS + 90))

  while (( SECONDS < deadline )); do
    if "$DRIVER" probe --keyboard-id "$keyboard_id" >/dev/null 2>&1 && \
      "$DRIVER" probe --keyboard-id "$secondary_keyboard_id" >/dev/null 2>&1; then
      break
    fi
    sleep 1
  done
  (( SECONDS < deadline )) || fail "Keyboards did not become ready after sign-in"

  local service_state overlay_pid
  while (( SECONDS < deadline )); do
    if service_state="$(launchctl print "gui/$(id -u)/$OVERLAY_LABEL" 2>/dev/null)"; then
      overlay_pid="$(awk '$1 == "pid" && $2 == "=" { print $3; exit }' <<<"$service_state")"
      if [[ "$overlay_pid" =~ ^[0-9]+$ ]]; then
        break
      fi
    fi
    sleep 1
  done
  [[ "${overlay_pid:-}" =~ ^[0-9]+$ ]] || fail "Overlay did not start after sign-in"

  printf 'PASS candidate=%s keyboard_id=%s secondary_keyboard_id=%s primary_layer=%s secondary_layer=%s signed_in_at=%s\n' \
    "$candidate" "$keyboard_id" "$secondary_keyboard_id" "$primary_layer" \
    "$secondary_layer" "$(date '+%Y-%m-%dT%H:%M:%S%z')" >"$RESULT"
}

verify() {
  local candidate keyboard_id secondary_keyboard_id primary_layer secondary_layer
  local service_state overlay_pid
  candidate="$(git -C "$ROOT" rev-parse HEAD)"
  [[ -f "$RESULT" ]] || {
    [[ ! -f "$STDERR_LOG" ]] || cat "$STDERR_LOG" >&2
    fail "No successful post-login result is available"
  }
  grep -Fq "PASS candidate=$candidate " "$RESULT" || \
    fail "The post-login result belongs to another candidate"

  keyboard_id="$(sed -n 's/.* keyboard_id=\([^ ]*\).*/\1/p' "$RESULT")"
  secondary_keyboard_id="$(sed -n 's/.* secondary_keyboard_id=\([^ ]*\).*/\1/p' "$RESULT")"
  primary_layer="$(sed -n 's/.* primary_layer=\([^ ]*\).*/\1/p' "$RESULT")"
  secondary_layer="$(sed -n 's/.* secondary_layer=\([^ ]*\).*/\1/p' "$RESULT")"
  [[ "$keyboard_id" =~ ^[0-9]+$ && "$secondary_keyboard_id" =~ ^[0-9]+$ && \
    "$primary_layer" =~ ^[0-9]+$ && "$secondary_layer" =~ ^[0-9]+$ ]] || \
    fail "The post-login result has invalid HIL parameters"

  service_state="$(launchctl print "gui/$(id -u)/$OVERLAY_LABEL")"
  overlay_pid="$(awk '$1 == "pid" && $2 == "=" { print $3; exit }' <<<"$service_state")"
  [[ "$overlay_pid" =~ ^[0-9]+$ ]] || fail "The installed overlay has no running PID"

  run_ui_probe "macOS Accessibility HIL checks passed" \
    --overlay-pid "$overlay_pid" \
    --driver "$DRIVER" \
    --keyboard-id "$keyboard_id" \
    --secondary-keyboard-id "$secondary_keyboard_id" \
    --layer "$primary_layer" \
    --secondary-layer "$secondary_layer" \
    --expected-label "L$primary_layer" \
    --skip-encoder-checks true
  printf 'VERIFIED candidate=%s verified_at=%s\n' \
    "$candidate" "$(date '+%Y-%m-%dT%H:%M:%S%z')" >>"$RESULT"
  cat "$RESULT"
  launchctl bootout "gui/$(id -u)/$LABEL" >/dev/null 2>&1 || true
  rm -f "$PLIST"
  printf 'PASS: actual sign-out/sign-in startup and interactive HIL layer event\n'
}

case "${1:-prepare}" in
  prepare) prepare "${2:-}" ;;
  after-login) after_login "$2" "$3" "$4" "$5" "$6" ;;
  verify) verify ;;
  *) fail "Expected prepare [--logout], after-login, or verify" ;;
esac

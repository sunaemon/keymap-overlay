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
HIL_LOG_DIR="${KMO_HIL_LOG_DIR:-$HOME/.local/var/log/keymap-overlay/hil}"
RESULT="$HIL_LOG_DIR/macos-login-result.txt"
STDOUT_LOG="$HIL_LOG_DIR/macos-login.out.log"
STDERR_LOG="$HIL_LOG_DIR/macos-login.err.log"
KEYBOARD_ID="${KMO_HIL_KEYBOARD_ID:-1}"
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
  output="$(mktemp "$HIL_LOG_DIR/macos-login-ui.XXXXXX.out")"
  error="$(mktemp "$HIL_LOG_DIR/macos-login-ui.XXXXXX.err")"
  if ! open -W -n -o "$output" --stderr "$error" "$UI_PROBE_APP" --args "$@"; then
    cat "$output" "$error"
    fail "LaunchServices could not run the HIL UI app"
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
  make -C "$ROOT" install-overlay

  rm -f "$RESULT" "$STDOUT_LOG" "$STDERR_LOG"
  cp "$TEMPLATE" "$PLIST"
  plutil -insert ProgramArguments.0 -string "$ROOT/overlay/platforms/macos/tests/test_hardware_login.sh" "$PLIST"
  plutil -insert ProgramArguments.1 -string after-login "$PLIST"
  plutil -insert ProgramArguments.2 -string "$(git -C "$ROOT" rev-parse HEAD)" "$PLIST"
  plutil -insert ProgramArguments.3 -string "$KEYBOARD_ID" "$PLIST"
  plutil -insert ProgramArguments.4 -string "$PRIMARY_LAYER" "$PLIST"
  plutil -insert ProgramArguments.5 -string "$SECONDARY_LAYER" "$PLIST"
  plutil -replace StandardOutPath -string "$STDOUT_LOG" "$PLIST"
  plutil -replace StandardErrorPath -string "$STDERR_LOG" "$PLIST"
  plutil -lint "$PLIST"

  printf 'Login continuation prepared for candidate %s.\n' "$(git -C "$ROOT" rev-parse HEAD)"
  printf 'The next sign-in will run the first-event and Accessibility checks.\n'
  if [[ "$request_logout" == --logout ]]; then
    osascript -e 'tell application "System Events" to log out'
  else
    printf 'Sign out and sign in, then run: make verify-hardware-login-macos\n'
  fi
}

after_login() {
  local candidate=$1
  local keyboard_id=$2
  local primary_layer=$3
  local secondary_layer=$4
  local deadline=$((SECONDS + 90))

  while (( SECONDS < deadline )); do
    if "$DRIVER" probe --keyboard-id "$keyboard_id" >/dev/null 2>&1; then
      break
    fi
    sleep 1
  done
  (( SECONDS < deadline )) || fail "Keyboard did not become ready after sign-in"

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

  run_ui_probe "macOS Accessibility HIL checks passed" \
    --overlay-pid "$overlay_pid" \
    --driver "$DRIVER" \
    --keyboard-id "$keyboard_id" \
    --layer "$primary_layer" \
    --secondary-layer "$secondary_layer" \
    --expected-label "L$primary_layer"
  printf 'PASS candidate=%s signed_in_at=%s\n' \
    "$candidate" "$(date '+%Y-%m-%dT%H:%M:%S%z')" >"$RESULT"
}

verify() {
  local candidate
  candidate="$(git -C "$ROOT" rev-parse HEAD)"
  [[ -f "$RESULT" ]] || {
    [[ ! -f "$STDERR_LOG" ]] || cat "$STDERR_LOG" >&2
    fail "No successful post-login result is available"
  }
  grep -Fq "PASS candidate=$candidate " "$RESULT" || \
    fail "The post-login result belongs to another candidate"
  cat "$RESULT"
  launchctl bootout "gui/$(id -u)/$LABEL" >/dev/null 2>&1 || true
  rm -f "$PLIST"
  printf 'PASS: actual sign-out/sign-in startup and first HIL layer event\n'
}

case "${1:-prepare}" in
  prepare) prepare "${2:-}" ;;
  after-login) after_login "$2" "$3" "$4" "$5" ;;
  verify) verify ;;
  *) fail "Expected prepare [--logout], after-login, or verify" ;;
esac

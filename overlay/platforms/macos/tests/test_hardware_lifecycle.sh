#!/bin/bash
# Copyright 2026 sunaemon
# SPDX-License-Identifier: MIT

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../../.." && pwd)"
MAIN_ROOT="$(git -C "$ROOT" worktree list --porcelain | awk '/^worktree / { sub(/^worktree /, ""); print; exit }')"
PREVIOUS_VERSION="${KMO_HIL_PREVIOUS_VERSION:-v0.0.7}"
PREVIOUS_WORKTREE="$MAIN_ROOT/.claude/worktrees/macos-lifecycle-$$"
DRIVER="$ROOT/target/release/keymap-overlay-hil"
KEYBOARD_ID="${KMO_HIL_KEYBOARD_ID:-1}"
LAYER="${KMO_HIL_PRIMARY_LAYER:-1}"
SERVICE_LABEL=com.sunaemon.keymap-overlay
PLIST="$HOME/Library/LaunchAgents/$SERVICE_LABEL.plist"
BINARY="$HOME/.local/bin/keymap-overlay"
LOG="$HOME/.local/var/log/keymap-overlay/overlay.log"
TRANSCRIPT_DIR="${KMO_HIL_LOG_DIR:-$HOME/.local/var/log/keymap-overlay/hil}"
TRANSCRIPT="$TRANSCRIPT_DIR/macos-lifecycle-$(date '+%Y%m%d-%H%M%S').log"

previous_worktree_added=false
candidate_installed=false

fail() {
  printf 'ERROR: %s\n' "$*" >&2
  exit 1
}

cleanup() {
  local status=$?
  set +e
  if ! $candidate_installed; then
    make -C "$ROOT" install-overlay
  fi
  if $previous_worktree_added; then
    git -C "$MAIN_ROOT" worktree remove --force "$PREVIOUS_WORKTREE"
  fi
  exit "$status"
}

wait_for_log() {
  local start_line=$1
  local pattern=$2
  local deadline=$((SECONDS + 10))
  while (( SECONDS < deadline )); do
    if [[ -f "$LOG" ]] && tail -n "+$start_line" "$LOG" | grep -Fq "$pattern"; then
      return
    fi
    sleep 0.1
  done
  fail "Timed out waiting for log entry: $pattern"
}

mkdir -p "$TRANSCRIPT_DIR"
exec > >(tee "$TRANSCRIPT") 2>&1
trap cleanup EXIT

[[ "$(uname -s)" == Darwin ]] || fail "This test requires macOS"
[[ "$(uname -m)" == arm64 ]] || fail "This release row requires macOS arm64"
[[ -z "$(git -C "$ROOT" status --short)" ]] || fail "Candidate worktree is not clean"
[[ ! -e "$PREVIOUS_WORKTREE" ]] || fail "Temporary lifecycle worktree already exists"

candidate_sha="$(git -C "$ROOT" rev-parse HEAD)"
printf 'Candidate: %s\n' "$candidate_sha"
make -C "$ROOT" build-hil-macos
"$DRIVER" probe --keyboard-id "$KEYBOARD_ID"

git -C "$MAIN_ROOT" worktree add --detach "$PREVIOUS_WORKTREE" "$PREVIOUS_VERSION"
previous_worktree_added=true
make -C "$PREVIOUS_WORKTREE" clean
make -C "$PREVIOUS_WORKTREE" install-overlay
candidate_installed=false

[[ -z "$(git -C "$ROOT" status --short)" ]] || fail "Candidate changed during previous install"
[[ "$(git -C "$ROOT" rev-parse HEAD)" == "$candidate_sha" ]] || \
  fail "Candidate SHA changed during upgrade setup"
make -C "$ROOT" clean
make -C "$ROOT" install-overlay
candidate_installed=true

launchctl print "gui/$(id -u)/$SERVICE_LABEL" >/dev/null
log_start="$(( $(wc -l <"$LOG") + 1 ))"
"$DRIVER" layer --keyboard-id "$KEYBOARD_ID" --layer "$LAYER" --state press
wait_for_log "$log_start" "show keyboard=$KEYBOARD_ID layers=[$LAYER]"
"$DRIVER" layer --keyboard-id "$KEYBOARD_ID" --layer "$LAYER" --state release
wait_for_log "$log_start" "hide size=1x1"
printf 'PASS: live upgrade from %s\n' "$PREVIOUS_VERSION"

make -C "$ROOT" test-release-acceptance-macos
printf 'PASS: local rollback acceptance\n'

make -C "$ROOT" uninstall-overlay
candidate_installed=false
[[ ! -e "$BINARY" ]] || fail "Overlay binary remains after uninstall"
[[ ! -e "$PLIST" ]] || fail "LaunchAgent remains after uninstall"
if launchctl print "gui/$(id -u)/$SERVICE_LABEL" >/dev/null 2>&1; then
  fail "Overlay service remains after uninstall"
fi
printf 'PASS: live uninstall\n'

make -C "$ROOT" install-overlay
candidate_installed=true
launchctl print "gui/$(id -u)/$SERVICE_LABEL" >/dev/null
printf 'PASS: candidate reinstalled after lifecycle checks\n'
printf 'Transcript: %s\n' "$TRANSCRIPT"

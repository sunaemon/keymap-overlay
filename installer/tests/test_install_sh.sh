#!/usr/bin/env sh
set -eu

PROJECT_DIRECTORY=$(CDPATH='' cd -- "$(dirname "$0")/../.." && pwd)
TEST_DIRECTORY=$(mktemp -d)
trap 'rm -rf "$TEST_DIRECTORY"' EXIT HUP INT TERM
REAL_INSTALL=$(command -v install)

FAKE_BIN="$TEST_DIRECTORY/bin"
FIXTURE_DIRECTORY="$TEST_DIRECTORY/fixture"
ARCHIVE="$TEST_DIRECTORY/release.tar.gz"
CHECKSUM='aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa'
mkdir -p "$FAKE_BIN" "$FIXTURE_DIRECTORY"

printf '#!/usr/bin/env sh\nexit 0\n' >"$FIXTURE_DIRECTORY/keymap-overlay"
printf '#!/usr/bin/env sh\nexit 0\n' >"$FIXTURE_DIRECTORY/keymap-overlay-qt"
printf 'MIT license fixture\n' >"$FIXTURE_DIRECTORY/LICENSE"
printf 'Third-party notices fixture\n' >"$FIXTURE_DIRECTORY/THIRD-PARTY-LICENSES.html"
mkdir -p "$FIXTURE_DIRECTORY/gnome-shell/keymap-overlay@sunaemon"
printf '{}\n' >"$FIXTURE_DIRECTORY/gnome-shell/keymap-overlay@sunaemon/metadata.json"
printf '// extension fixture\n' >"$FIXTURE_DIRECTORY/gnome-shell/keymap-overlay@sunaemon/extension.js"
printf '/* stylesheet fixture */\n' >"$FIXTURE_DIRECTORY/gnome-shell/keymap-overlay@sunaemon/stylesheet.css"
chmod +x "$FIXTURE_DIRECTORY/keymap-overlay" "$FIXTURE_DIRECTORY/keymap-overlay-qt"
tar -C "$FIXTURE_DIRECTORY" -czf "$ARCHIVE" keymap-overlay keymap-overlay-qt gnome-shell LICENSE THIRD-PARTY-LICENSES.html

for command in awk cat cp dirname find grep gzip id mkdir mktemp mv rm tar; do
  ln -s "$(command -v "$command")" "$FAKE_BIN/$command"
done

cat >"$FAKE_BIN/uname" <<'EOF'
#!/bin/sh
case "$1" in
  -s) printf '%s\n' "$TEST_UNAME_S" ;;
  -m) printf '%s\n' "$TEST_UNAME_M" ;;
esac
EOF

cat >"$FAKE_BIN/curl" <<'EOF'
#!/bin/sh
url=''
output=''
while [ "$#" -gt 0 ]; do
  case "$1" in
    -o)
      output=$2
      shift 2
      ;;
    http*)
      url=$1
      shift
      ;;
    *) shift ;;
  esac
done
case "$url" in
  */releases/latest)
    printf '%s\n' 'https://github.com/sunaemon/keymap-overlay/releases/tag/v0.0.1'
    ;;
  */SHA256SUMS)
    printf '%s  %s\n' "$TEST_CHECKSUM" "$TEST_ASSET_NAME" >"$output"
    printf '%s  install.sh\n' "$TEST_CHECKSUM" >>"$output"
    ;;
  */install.sh)
    cp "$TEST_INSTALLER" "$output"
    ;;
  *)
    cp "$TEST_ARCHIVE" "$output"
    ;;
esac
printf '%s\n' "$url" >>"$TEST_CURL_LOG"
EOF

cat >"$FAKE_BIN/sha256sum" <<'EOF'
#!/bin/sh
printf '%s  %s\n' "$TEST_CHECKSUM" "$1"
EOF

cat >"$FAKE_BIN/shasum" <<'EOF'
#!/bin/sh
shift 2
printf '%s  %s\n' "$TEST_CHECKSUM" "$1"
EOF

cat >"$FAKE_BIN/install" <<'EOF'
#!/bin/sh
printf 'install %s\n' "$*" >>"$TEST_COMMAND_LOG"
exec "$TEST_REAL_INSTALL" "$@"
EOF

cat >"$FAKE_BIN/systemctl" <<'EOF'
#!/bin/sh
printf 'systemctl %s\n' "$*" >>"$TEST_COMMAND_LOG"
case "$*" in
  *is-enabled*keymap-overlay-qt*) [ "${TEST_QT_ENABLED:-1}" -eq 1 ] ;;
  *is-active*keymap-overlay-qt*) [ "${TEST_QT_ACTIVE:-1}" -eq 1 ] ;;
  *is-enabled*keymap-overlay.service*) [ "${TEST_MAIN_ENABLED:-1}" -eq 1 ] ;;
  *is-active*keymap-overlay.service*) [ "${TEST_MAIN_ACTIVE:-1}" -eq 1 ] ;;
  *restart*keymap-overlay.service*)
    if [ "${TEST_FAIL_SERVICE:-0}" -ne 0 ] && [ ! -f "${TEST_COMMAND_LOG}.failed-once" ]; then
      : >"${TEST_COMMAND_LOG}.failed-once"
      exit 1
    fi
    ;;
  *) exit 0 ;;
esac
EOF

cat >"$FAKE_BIN/launchctl" <<'EOF'
#!/bin/sh
printf 'launchctl %s\n' "$*" >>"$TEST_COMMAND_LOG"
exit 0
EOF

cat >"$FAKE_BIN/gh" <<'EOF'
#!/bin/sh
case "$*" in
  'auth status') exit 1 ;;
  *) echo "Unexpected unauthenticated gh invocation: $*" >&2; exit 1 ;;
esac
EOF

chmod +x \
  "$FAKE_BIN/uname" \
  "$FAKE_BIN/curl" \
  "$FAKE_BIN/sha256sum" \
  "$FAKE_BIN/shasum" \
  "$FAKE_BIN/install" \
  "$FAKE_BIN/systemctl" \
  "$FAKE_BIN/launchctl" \
  "$FAKE_BIN/gh"

assert_file_contains() {
  file=$1
  expected=$2
  if ! grep -F "$expected" "$file" >/dev/null; then
    echo "Expected $file to contain: $expected" >&2
    exit 1
  fi
}

run_installer() {
  home=$1
  os=$2
  arch=$3
  asset=$4
  shift 4
  PATH="$FAKE_BIN" \
    HOME="$home" \
    TEST_UNAME_S="$os" \
    TEST_UNAME_M="$arch" \
    TEST_ASSET_NAME="$asset" \
    TEST_ARCHIVE="$ARCHIVE" \
    TEST_INSTALLER="$PROJECT_DIRECTORY/installer/install.sh" \
    TEST_CHECKSUM="$CHECKSUM" \
    TEST_COMMAND_LOG="$home/commands.log" \
    TEST_CURL_LOG="$home/curl.log" \
    TEST_REAL_INSTALL="$REAL_INSTALL" \
    TEST_FAIL_SERVICE="${TEST_FAIL_SERVICE:-0}" \
    TEST_MAIN_ENABLED="${TEST_MAIN_ENABLED:-1}" \
    TEST_MAIN_ACTIVE="${TEST_MAIN_ACTIVE:-1}" \
    TEST_QT_ENABLED="${TEST_QT_ENABLED:-1}" \
    TEST_QT_ACTIVE="${TEST_QT_ACTIVE:-1}" \
    XDG_CURRENT_DESKTOP="${TEST_XDG_CURRENT_DESKTOP:-}" \
    KEYMAP_OVERLAY_FORCE_QT="${TEST_KEYMAP_OVERLAY_FORCE_QT:-}" \
    /bin/sh "$PROJECT_DIRECTORY/installer/install.sh" "$@"
}

test_linux_install_and_uninstall() {
  home="$TEST_DIRECTORY/linux home"
  cache="$home/.cache/keymap-overlay"
  state="$home/.config/keymap-overlay"
  bin="$home/.local/bin"
  mkdir -p "$cache"
  mkdir -p "$state/keyboards/custom"
  printf 'user config\n' >"$state/keyboards/custom/config.json"
  : >"$cache/1.json"
  : >"$home/commands.log"

  run_installer "$home" Linux x86_64 keymap-overlay-linux-x86_64.tar.gz

  test -x "$bin/keymap-overlay"
  test ! -e "$bin/keymap-overlay-generator"
  test -x "$bin/keymap-overlay-qt"
  test -x "$state/install.sh"
  test ! -e "$state/keyboards/1"
  assert_file_contains "$state/keyboards/custom/config.json" 'user config'
  test ! -e "$state/GENERATOR-THIRD-PARTY-LICENSES.html"
  # The binary embeds the notices, so nothing is installed for them.
  test ! -e "$state/LICENSE"
  test ! -e "$state/THIRD-PARTY-LICENSES.html"
  unit="$home/.config/systemd/user/keymap-overlay.service"
  qt_unit="$home/.config/systemd/user/keymap-overlay-qt.service"
  extension="$home/.local/share/gnome-shell/extensions/keymap-overlay@sunaemon"
  assert_file_contains "$unit" "ExecStart=\"$bin/keymap-overlay\""
  if grep -E -- '--(asset-dir|keyboard-config-dir)' "$unit" >/dev/null; then
    echo 'The systemd unit should not select persistent model input.' >&2
    exit 1
  fi
  # Anchored on the directives: the unit's own comments mention both names.
  assert_file_contains "$unit" "SyslogIdentifier=keymap-overlay"
  if grep -q '^ExecStart=.*--log-out' "$unit"; then
    echo 'The systemd unit should not name a log file.' >&2
    exit 1
  fi
  if grep -q '^Environment=' "$unit"; then
    echo 'The systemd unit should not set a log directory.' >&2
    exit 1
  fi
  assert_file_contains "$qt_unit" "ExecStart=\"$bin/keymap-overlay-qt\""
  test -f "$extension/metadata.json"
  test -f "$extension/extension.js"
  test -f "$extension/stylesheet.css"

  run_installer "$home" Linux x86_64 keymap-overlay-linux-x86_64.tar.gz uninstall
  test ! -e "$bin/keymap-overlay"
  test ! -e "$bin/keymap-overlay-generator"
  test ! -e "$bin/keymap-overlay-qt"
  test ! -e "$state/install.sh"
  assert_file_contains "$state/keyboards/custom/config.json" 'user config'
  test ! -e "$state/GENERATOR-THIRD-PARTY-LICENSES.html"
  test ! -e "$unit"
  test ! -e "$qt_unit"
  test ! -e "$extension"
  test ! -e "$cache"
  test -d "$home/.local/var/log/keymap-overlay"
}

test_linux_arm64_install() {
  home="$TEST_DIRECTORY/linux arm64 home"
  mkdir -p "$home/.cache/keymap-overlay"
  : >"$home/commands.log"

  run_installer "$home" Linux aarch64 keymap-overlay-linux-arm64.tar.gz

  assert_file_contains "$home/curl.log" 'keymap-overlay-linux-arm64.tar.gz'
}

test_macos_stops_service_before_replacing_binary() {
  home="$TEST_DIRECTORY/macos & home"
  cache="$home/.cache/keymap-overlay"
  mkdir -p "$cache"
  : >"$cache/1.json"
  : >"$home/commands.log"

  run_installer "$home" Darwin arm64 keymap-overlay-macos-arm64.tar.gz

  test -d "$home/.local/var/log/keymap-overlay"
  plist="$home/Library/LaunchAgents/com.sunaemon.keymap-overlay.plist"
  # The ampersand in the home directory must survive XML escaping.
  if grep -E -- '--(asset-dir|keyboard-config-dir)' "$plist" >/dev/null; then
    echo 'The launchd plist should not select persistent model input.' >&2
    exit 1
  fi
  assert_file_contains "$plist" '<string>--log-out</string>'
  assert_file_contains "$plist" 'macos &amp; home/.local/var/log/keymap-overlay/overlay.log'
  if grep -F 'KEYMAP_OVERLAY_LOG_DIR' "$plist" >/dev/null; then
    echo 'The launchd plist should pass the log path as an argument.' >&2
    exit 1
  fi
  stop_line=$(grep -n 'launchctl bootout' "$home/commands.log" | awk -F: 'NR == 1 { print $1 }')
  install_line=$(grep -n 'install .*keymap-overlay' "$home/commands.log" | awk -F: 'NR == 1 { print $1 }')
  test "$stop_line" -lt "$install_line"
}

test_gnome_disables_qt_renderer_unless_forced() {
  home="$TEST_DIRECTORY/gnome home"
  cache="$home/.cache/keymap-overlay"
  mkdir -p "$cache"
  : >"$cache/1.json"
  : >"$home/commands.log"

  TEST_XDG_CURRENT_DESKTOP=ubuntu:GNOME \
    run_installer "$home" Linux x86_64 keymap-overlay-linux-x86_64.tar.gz

  grep -F 'systemctl --user disable --now keymap-overlay-qt.service' "$home/commands.log" >/dev/null
  if grep -F 'systemctl --user restart keymap-overlay-qt.service' "$home/commands.log" >/dev/null; then
    echo 'Qt renderer unexpectedly started under GNOME.' >&2
    exit 1
  fi

  : >"$home/commands.log"
  TEST_XDG_CURRENT_DESKTOP=ubuntu:GNOME \
    TEST_KEYMAP_OVERLAY_FORCE_QT=1 \
    run_installer "$home" Linux x86_64 keymap-overlay-linux-x86_64.tar.gz
  grep -F 'systemctl --user restart keymap-overlay-qt.service' "$home/commands.log" >/dev/null
}

test_failed_service_install_rolls_back() {
  home="$TEST_DIRECTORY/rollback home"
  cache="$home/.cache/keymap-overlay"
  state="$home/.config/keymap-overlay"
  bin="$home/.local/bin"
  unit="$home/.config/systemd/user/keymap-overlay.service"
  mkdir -p "$cache" "$state" "$bin" "$(dirname "$unit")"
  : >"$cache/1.json"
  printf 'old binary\n' >"$bin/keymap-overlay"
  printf 'old generator\n' >"$bin/keymap-overlay-generator"
  printf 'old generator notices\n' >"$state/GENERATOR-THIRD-PARTY-LICENSES.html"
  printf 'old qt binary\n' >"$bin/keymap-overlay-qt"
  mkdir -p "$state/keyboards/custom"
  printf 'user config\n' >"$state/keyboards/custom/config.json"
  printf 'old installer\n' >"$state/install.sh"
  printf 'old service\n' >"$unit"
  : >"$home/commands.log"

  if TEST_FAIL_SERVICE=1 run_installer "$home" Linux x86_64 keymap-overlay-linux-x86_64.tar.gz; then
    echo 'Expected service installation to fail.' >&2
    exit 1
  fi

  assert_file_contains "$bin/keymap-overlay" 'old binary'
  assert_file_contains "$bin/keymap-overlay-generator" 'old generator'
  assert_file_contains "$state/GENERATOR-THIRD-PARTY-LICENSES.html" 'old generator notices'
  assert_file_contains "$bin/keymap-overlay-qt" 'old qt binary'
  assert_file_contains "$state/keyboards/custom/config.json" 'user config'
  assert_file_contains "$state/install.sh" 'old installer'
  assert_file_contains "$unit" 'old service'
  enable_count=$(grep -c '^systemctl --user enable keymap-overlay.service' "$home/commands.log")
  test "$enable_count" -eq 2
}

test_failed_gnome_upgrade_keeps_qt_disabled() {
  home="$TEST_DIRECTORY/gnome rollback home"
  cache="$home/.cache/keymap-overlay"
  unit_directory="$home/.config/systemd/user"
  mkdir -p "$cache" "$unit_directory"
  : >"$cache/1.json"
  printf 'old binary\n' >"$cache/keymap-overlay"
  printf 'old service\n' >"$unit_directory/keymap-overlay.service"
  printf 'old qt service\n' >"$unit_directory/keymap-overlay-qt.service"
  : >"$home/commands.log"

  if TEST_FAIL_SERVICE=1 \
    TEST_MAIN_ENABLED=0 \
    TEST_MAIN_ACTIVE=0 \
    TEST_QT_ENABLED=0 \
    TEST_QT_ACTIVE=0 \
    TEST_XDG_CURRENT_DESKTOP=GNOME \
    run_installer "$home" Linux x86_64 keymap-overlay-linux-x86_64.tar.gz; then
    echo 'Expected GNOME service installation to fail.' >&2
    exit 1
  fi

  if grep -F 'systemctl --user enable keymap-overlay-qt.service' "$home/commands.log" >/dev/null; then
    echo 'Rollback unexpectedly enabled the previously disabled Qt renderer.' >&2
    exit 1
  fi
  grep -F 'systemctl --user disable keymap-overlay-qt.service' "$home/commands.log" >/dev/null
  grep -F 'systemctl --user stop keymap-overlay-qt.service' "$home/commands.log" >/dev/null
  grep -F 'systemctl --user disable keymap-overlay.service' "$home/commands.log" >/dev/null
  grep -F 'systemctl --user stop keymap-overlay.service' "$home/commands.log" >/dev/null
}

test_install_does_not_select_example_keyboard_config() {
  home="$TEST_DIRECTORY/no assets home"
  cache="$home/.cache/keymap-overlay"
  mkdir -p "$cache"
  : >"$home/commands.log"

  run_installer "$home" Linux x86_64 keymap-overlay-linux-x86_64.tar.gz

  test -x "$home/.local/bin/keymap-overlay"
  test ! -e "$home/.local/bin/keymap-overlay-generator"
  test ! -e "$home/.config/keymap-overlay/keyboards"
  test -f "$home/.config/systemd/user/keymap-overlay.service"
}

test_linux_install_and_uninstall
test_linux_arm64_install
test_macos_stops_service_before_replacing_binary
test_gnome_disables_qt_renderer_unless_forced
test_failed_service_install_rolls_back
test_failed_gnome_upgrade_keeps_qt_disabled
test_install_does_not_select_example_keyboard_config
echo 'install.sh tests passed'

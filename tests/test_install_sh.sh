#!/usr/bin/env sh
set -eu

PROJECT_DIRECTORY=$(CDPATH='' cd -- "$(dirname "$0")/.." && pwd)
TEST_DIRECTORY=$(mktemp -d)
trap 'rm -rf "$TEST_DIRECTORY"' EXIT HUP INT TERM
REAL_INSTALL=$(command -v install)

FAKE_BIN="$TEST_DIRECTORY/bin"
FIXTURE_DIRECTORY="$TEST_DIRECTORY/fixture"
ARCHIVE="$TEST_DIRECTORY/release.tar.gz"
CHECKSUM='aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa'
mkdir -p "$FAKE_BIN" "$FIXTURE_DIRECTORY"

printf '#!/usr/bin/env sh\nexit 0\n' >"$FIXTURE_DIRECTORY/keymap-overlay"
printf 'MIT license fixture\n' >"$FIXTURE_DIRECTORY/LICENSE"
printf 'Third-party notices fixture\n' >"$FIXTURE_DIRECTORY/THIRD-PARTY-LICENSES.html"
chmod +x "$FIXTURE_DIRECTORY/keymap-overlay"
tar -C "$FIXTURE_DIRECTORY" -czf "$ARCHIVE" keymap-overlay LICENSE THIRD-PARTY-LICENSES.html

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
  *restart*keymap-overlay*) [ "${TEST_FAIL_SERVICE:-0}" -eq 0 ] ;;
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
    TEST_INSTALLER="$PROJECT_DIRECTORY/install.sh" \
    TEST_CHECKSUM="$CHECKSUM" \
    TEST_COMMAND_LOG="$home/commands.log" \
    TEST_REAL_INSTALL="$REAL_INSTALL" \
    TEST_FAIL_SERVICE="${TEST_FAIL_SERVICE:-0}" \
    /bin/sh "$PROJECT_DIRECTORY/install.sh" "$@"
}

test_linux_install_and_uninstall() {
  home="$TEST_DIRECTORY/linux home"
  assets="$home/.config/keymap-overlay"
  mkdir -p "$assets"
  : >"$assets/1_L0.png"
  : >"$home/commands.log"

  run_installer "$home" Linux x86_64 keymap-overlay-linux-x86_64.tar.gz

  test -x "$assets/keymap-overlay"
  test -f "$assets/LICENSE"
  test -f "$assets/THIRD-PARTY-LICENSES.html"
  test -x "$assets/install.sh"
  unit="$home/.config/systemd/user/keymap-overlay.service"
  assert_file_contains "$unit" "ExecStart=\"$assets/keymap-overlay\" \"$assets\""
  assert_file_contains "$unit" "Environment=\"KEYMAP_OVERLAY_LOG_DIR=$home/.local/var/log/keymap-overlay\""

  run_installer "$home" Linux x86_64 keymap-overlay-linux-x86_64.tar.gz uninstall
  test ! -e "$assets/keymap-overlay"
  test ! -e "$assets/install.sh"
  test ! -e "$unit"
  test -f "$assets/1_L0.png"
  test -d "$home/.local/var/log/keymap-overlay"
}

test_macos_stops_service_before_replacing_binary() {
  home="$TEST_DIRECTORY/macos home"
  assets="$home/.config/keymap-overlay"
  mkdir -p "$assets"
  : >"$assets/1_L0.png"
  : >"$home/commands.log"

  run_installer "$home" Darwin arm64 keymap-overlay-macos-arm64.tar.gz

  stop_line=$(grep -n 'launchctl bootout' "$home/commands.log" | awk -F: 'NR == 1 { print $1 }')
  install_line=$(grep -n 'install .*keymap-overlay' "$home/commands.log" | awk -F: 'NR == 1 { print $1 }')
  test "$stop_line" -lt "$install_line"
}

test_failed_service_install_rolls_back() {
  home="$TEST_DIRECTORY/rollback home"
  assets="$home/.config/keymap-overlay"
  unit="$home/.config/systemd/user/keymap-overlay.service"
  mkdir -p "$assets" "$(dirname "$unit")"
  : >"$assets/1_L0.png"
  printf 'old binary\n' >"$assets/keymap-overlay"
  printf 'old license\n' >"$assets/LICENSE"
  printf 'old notices\n' >"$assets/THIRD-PARTY-LICENSES.html"
  printf 'old installer\n' >"$assets/install.sh"
  printf 'old service\n' >"$unit"
  : >"$home/commands.log"

  if TEST_FAIL_SERVICE=1 run_installer "$home" Linux x86_64 keymap-overlay-linux-x86_64.tar.gz; then
    echo 'Expected service installation to fail.' >&2
    exit 1
  fi

  assert_file_contains "$assets/keymap-overlay" 'old binary'
  assert_file_contains "$assets/LICENSE" 'old license'
  assert_file_contains "$assets/THIRD-PARTY-LICENSES.html" 'old notices'
  assert_file_contains "$assets/install.sh" 'old installer'
  assert_file_contains "$unit" 'old service'
  enable_count=$(grep -c '^systemctl --user enable keymap-overlay.service$' "$home/commands.log")
  test "$enable_count" -eq 2
}

test_missing_layer_assets_fails_without_installing_files() {
  home="$TEST_DIRECTORY/no assets home"
  assets="$home/.config/keymap-overlay"
  mkdir -p "$assets"
  : >"$home/commands.log"

  if run_installer "$home" Linux x86_64 keymap-overlay-linux-x86_64.tar.gz; then
    echo 'Expected installation without layer assets to fail.' >&2
    exit 1
  fi

  test ! -e "$assets/keymap-overlay"
  test ! -e "$assets/LICENSE"
  test ! -e "$assets/THIRD-PARTY-LICENSES.html"
  test ! -e "$assets/install.sh"
  test ! -e "$home/.config/systemd/user/keymap-overlay.service"
}

test_linux_install_and_uninstall
test_macos_stops_service_before_replacing_binary
test_failed_service_install_rolls_back
test_missing_layer_assets_fails_without_installing_files
echo 'install.sh tests passed'

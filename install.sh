#!/usr/bin/env sh
# Installs the latest keymap-overlay release for macOS or Linux.
set -eu

REPOSITORY='sunaemon/keymap-overlay'
ASSET_DIRECTORY="${HOME}/.config/keymap-overlay"
BINARY_PATH="${ASSET_DIRECTORY}/keymap-overlay"
LOG_DIRECTORY="${HOME}/.local/var/log/keymap-overlay"

require_command() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "ERROR: install.sh requires '$1'." >&2
    exit 1
  fi
}

install_binary() {
  archive="${temporary_directory}/${asset_name}"
  curl -fsSL "https://github.com/${REPOSITORY}/releases/latest/download/${asset_name}" -o "$archive"
  tar -xzf "$archive" -C "$temporary_directory"
  install -m 755 "${temporary_directory}/keymap-overlay" "$BINARY_PATH"
}

install_macos_service() {
  label='com.sunaemon.keymap-overlay'
  plist="${HOME}/Library/LaunchAgents/${label}.plist"

  mkdir -p "$(dirname "$plist")"
  cat >"${plist}.tmp" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>${label}</string>
  <key>ProgramArguments</key>
  <array>
    <string>${BINARY_PATH}</string>
    <string>${ASSET_DIRECTORY}</string>
  </array>
  <key>RunAtLoad</key>
  <true/>
  <key>KeepAlive</key>
  <dict><key>SuccessfulExit</key><false/></dict>
  <key>ProcessType</key>
  <string>Interactive</string>
  <key>EnvironmentVariables</key>
  <dict>
    <key>KEYMAP_OVERLAY_LOG_DIR</key>
    <string>${LOG_DIRECTORY}</string>
  </dict>
</dict>
</plist>
EOF
  mv "${plist}.tmp" "$plist"
  launchctl bootout "gui/$(id -u)/${label}" 2>/dev/null || true
  launchctl bootstrap "gui/$(id -u)" "$plist"
}

install_linux_service() {
  unit_directory="${HOME}/.config/systemd/user"
  unit="${unit_directory}/keymap-overlay.service"

  mkdir -p "$unit_directory"
  cat >"${unit}.tmp" <<EOF
[Unit]
Description=QMK keymap layer overlay
Documentation=https://github.com/${REPOSITORY}
PartOf=graphical-session.target
After=graphical-session.target
StartLimitIntervalSec=0

[Service]
Type=simple
ExecStart=${BINARY_PATH} ${ASSET_DIRECTORY}
Environment=KEYMAP_OVERLAY_LOG_DIR=${LOG_DIRECTORY}
Restart=on-failure
RestartSec=2

[Install]
WantedBy=graphical-session.target
EOF
  mv "${unit}.tmp" "$unit"
  systemctl --user daemon-reload
  systemctl --user enable keymap-overlay.service
  systemctl --user restart keymap-overlay.service
}

require_command curl
require_command install
require_command tar

if [ ! -d "$ASSET_DIRECTORY" ] || ! find "$ASSET_DIRECTORY" -maxdepth 1 -type f -name '*.png' -print -quit | grep -q .; then
  echo "ERROR: no layer PNGs found in ${ASSET_DIRECTORY}." >&2
  echo "Generate assets from a source checkout before installing the binary." >&2
  exit 1
fi

case "$(uname -s):$(uname -m)" in
  Darwin:arm64)
    asset_name='keymap-overlay-macos-arm64.tar.gz'
    service_installer=install_macos_service
    ;;
  Linux:x86_64)
    asset_name='keymap-overlay-linux-x86_64.tar.gz'
    service_installer=install_linux_service
    ;;
  *)
    echo "ERROR: no release binary is available for $(uname -s) $(uname -m)." >&2
    exit 1
    ;;
esac

temporary_directory="$(mktemp -d)"
trap 'rm -rf "$temporary_directory"' EXIT HUP INT TERM
mkdir -p "$ASSET_DIRECTORY" "$LOG_DIRECTORY"
install_binary
"$service_installer"
echo "Installed the latest release to ${BINARY_PATH}."

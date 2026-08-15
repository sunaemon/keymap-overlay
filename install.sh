#!/usr/bin/env sh
# Installs or removes a verified keymap-overlay release on macOS or Linux.
set -eu

REPOSITORY='sunaemon/keymap-overlay'
ASSET_DIRECTORY="${HOME}/.config/keymap-overlay"
BINARY_PATH="${ASSET_DIRECTORY}/keymap-overlay"
LICENSE_PATH="${ASSET_DIRECTORY}/LICENSE"
THIRD_PARTY_LICENSES_PATH="${ASSET_DIRECTORY}/THIRD-PARTY-LICENSES.html"
INSTALLER_PATH="${ASSET_DIRECTORY}/install.sh"
LOG_DIRECTORY="${HOME}/.local/var/log/keymap-overlay"

main() {
  configure_platform

  case "${1:-install}" in
    install)
      install_release
      ;;
    uninstall|--uninstall)
      uninstall_release
      ;;
    *)
      echo "Usage: $0 [install|uninstall]" >&2
      exit 2
      ;;
  esac
}

install_release() {
  require_command curl
  require_command install
  require_command tar
  require_command "$checksum_command"
  require_layer_assets

  temporary_directory="$(mktemp -d)"
  trap 'rm -rf "$temporary_directory"' EXIT HUP INT TERM
  stage_release
  mkdir -p "$ASSET_DIRECTORY" "$LOG_DIRECTORY"
  backup_installation
  stop_service

  if install_staged_files && install_service; then
    print_installed_files
    return
  fi

  echo 'ERROR: installation failed; restoring the previous installation.' >&2
  "$service_uninstaller" || true
  restore_installation
  restart_previous_service
  exit 1
}

uninstall_release() {
  stop_and_remove_service
  rm -f "$BINARY_PATH" "$LICENSE_PATH" "$THIRD_PARTY_LICENSES_PATH" "$INSTALLER_PATH"

  echo 'Removed:'
  echo "  binary: ${BINARY_PATH}"
  echo "  licenses: ${LICENSE_PATH}, ${THIRD_PARTY_LICENSES_PATH}"
  echo "  installer: ${INSTALLER_PATH}"
  echo "  autostart: ${service_path}"
  echo "Kept layer assets: ${ASSET_DIRECTORY}"
  echo "Kept logs: ${LOG_DIRECTORY}"
}

configure_platform() {
  case "$(uname -s):$(uname -m)" in
    Darwin:arm64)
      asset_name='keymap-overlay-macos-arm64.tar.gz'
      asset_extension='json'
      checksum_command='shasum'
      service_path="${HOME}/Library/LaunchAgents/com.sunaemon.keymap-overlay.plist"
      service_installer=install_macos_service
      service_stopper=stop_macos_service
      service_uninstaller=uninstall_macos_service
      previous_service_restarter=restart_previous_macos_service
      ;;
    Linux:x86_64)
      asset_name='keymap-overlay-linux-x86_64.tar.gz'
      asset_extension='json'
      checksum_command='sha256sum'
      service_path="${HOME}/.config/systemd/user/keymap-overlay.service"
      service_installer=install_linux_service
      service_stopper=stop_linux_service
      service_uninstaller=uninstall_linux_service
      previous_service_restarter=restart_previous_linux_service
      ;;
    *)
      echo "ERROR: no release binary is available for $(uname -s) $(uname -m)." >&2
      exit 1
      ;;
  esac
}

require_command() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "ERROR: install.sh requires '$1'." >&2
    exit 1
  fi
}

require_layer_assets() {
  if [ ! -d "$ASSET_DIRECTORY" ] ||
    ! find "$ASSET_DIRECTORY" -maxdepth 1 -type f -name "*.${asset_extension}" -print -quit | grep -q .; then
    echo "ERROR: no layer ${asset_extension} assets found in ${ASSET_DIRECTORY}." >&2
    echo 'Generate assets from a source checkout before installing the binary.' >&2
    exit 1
  fi
}

stage_release() {
  latest_url="$(curl -fsSL -o /dev/null -w '%{url_effective}' "https://github.com/${REPOSITORY}/releases/latest")"
  release_tag=${latest_url##*/}
  if ! printf '%s\n' "$release_tag" | grep -Eq '^v[0-9]+\.[0-9]+\.[0-9]+$'; then
    echo "ERROR: latest release returned invalid tag '${release_tag}'." >&2
    exit 1
  fi

  release_url="https://github.com/${REPOSITORY}/releases/download/${release_tag}"
  archive="${temporary_directory}/${asset_name}"
  checksums="${temporary_directory}/SHA256SUMS"
  staged_installer="${temporary_directory}/release-install.sh"
  curl -fsSL "${release_url}/${asset_name}" -o "$archive"
  curl -fsSL "${release_url}/SHA256SUMS" -o "$checksums"
  curl -fsSL "${release_url}/install.sh" -o "$staged_installer"
  verify_checksum "$archive" "$checksums" "$asset_name"
  verify_checksum "$staged_installer" "$checksums" install.sh
  verify_attestations_if_available "$archive" "$checksums" "$staged_installer"
  tar -xzf "$archive" -C "$temporary_directory"

  for file in keymap-overlay LICENSE THIRD-PARTY-LICENSES.html; do
    if [ ! -f "${temporary_directory}/${file}" ]; then
      echo "ERROR: ${asset_name} does not contain ${file}." >&2
      exit 1
    fi
  done
}

verify_attestations_if_available() {
  if command -v gh >/dev/null 2>&1 && gh auth status >/dev/null 2>&1; then
    for file in "$@"; do
      gh attestation verify "$file" --repo "$REPOSITORY"
    done
  else
    echo "NOTE: SHA-256 verified; install and authenticate GitHub CLI to also verify artifact provenance."
  fi
}

verify_checksum() {
  file=$1
  manifest=$2
  name=$3
  expected="$(awk -v name="$name" '$2 == name || $2 == "*" name { print $1 }' "$manifest")"
  case "$expected" in
    ''|*[!0-9a-fA-F]*)
      echo "ERROR: SHA256SUMS has no checksum for ${name}." >&2
      exit 1
      ;;
  esac
  if [ "${#expected}" -ne 64 ]; then
    echo "ERROR: SHA256SUMS has an invalid checksum for ${name}." >&2
    exit 1
  fi

  if [ "$checksum_command" = 'shasum' ]; then
    actual="$(shasum -a 256 "$file" | awk '{ print $1 }')"
  else
    actual="$(sha256sum "$file" | awk '{ print $1 }')"
  fi
  if [ "$actual" != "$expected" ]; then
    echo "ERROR: SHA-256 verification failed for ${name}." >&2
    exit 1
  fi
}

backup_installation() {
  backup_file "$BINARY_PATH" binary
  backup_file "$LICENSE_PATH" license
  backup_file "$THIRD_PARTY_LICENSES_PATH" third-party-licenses
  backup_file "$INSTALLER_PATH" installer
  backup_file "$service_path" service
}

backup_file() {
  source_path=$1
  backup_name=$2
  if [ -f "$source_path" ]; then
    cp -p "$source_path" "${temporary_directory}/backup-${backup_name}"
    : >"${temporary_directory}/had-${backup_name}"
  fi
}

stop_service() {
  "$service_stopper"
}

install_staged_files() {
    install -m 755 "${temporary_directory}/keymap-overlay" "$BINARY_PATH" &&
    install -m 644 "${temporary_directory}/LICENSE" "$LICENSE_PATH" &&
    install -m 644 "${temporary_directory}/THIRD-PARTY-LICENSES.html" "$THIRD_PARTY_LICENSES_PATH" &&
    install -m 755 "$staged_installer" "$INSTALLER_PATH"
}

install_service() {
  "$service_installer"
}

restore_installation() {
  restore_file "$BINARY_PATH" binary
  restore_file "$LICENSE_PATH" license
  restore_file "$THIRD_PARTY_LICENSES_PATH" third-party-licenses
  restore_file "$INSTALLER_PATH" installer
  restore_file "$service_path" service
}

restore_file() {
  destination=$1
  backup_name=$2
  if [ -f "${temporary_directory}/had-${backup_name}" ]; then
    mkdir -p "$(dirname "$destination")"
    cp -p "${temporary_directory}/backup-${backup_name}" "$destination"
  else
    rm -f "$destination"
  fi
}

restart_previous_service() {
  if [ -f "${temporary_directory}/had-service" ]; then
    "$previous_service_restarter" || true
  fi
}

install_macos_service() {
  label='com.sunaemon.keymap-overlay'
  mkdir -p "$(dirname "$service_path")" || return
  cat >"${service_path}.tmp" <<EOF
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
  mv "${service_path}.tmp" "$service_path" || return
  launchctl bootstrap "gui/$(id -u)" "$service_path"
}

stop_macos_service() {
  launchctl bootout 'gui/'"$(id -u)"'/com.sunaemon.keymap-overlay' 2>/dev/null || true
}

uninstall_macos_service() {
  stop_macos_service
  rm -f "$service_path"
}

restart_previous_macos_service() {
  launchctl bootstrap "gui/$(id -u)" "$service_path"
}

install_linux_service() {
  mkdir -p "$(dirname "$service_path")" || return
  cat >"${service_path}.tmp" <<EOF
[Unit]
Description=QMK keymap layer overlay
Documentation=https://github.com/${REPOSITORY}
PartOf=graphical-session.target
After=graphical-session.target
StartLimitIntervalSec=0

[Service]
Type=simple
ExecStart="${BINARY_PATH}" "${ASSET_DIRECTORY}"
Environment="KEYMAP_OVERLAY_LOG_DIR=${LOG_DIRECTORY}"
Restart=on-failure
RestartSec=2

[Install]
WantedBy=graphical-session.target
EOF
  mv "${service_path}.tmp" "$service_path" || return
  systemctl --user daemon-reload &&
    systemctl --user enable keymap-overlay.service &&
    systemctl --user restart keymap-overlay.service
}

stop_linux_service() {
  systemctl --user stop keymap-overlay.service 2>/dev/null || true
}

uninstall_linux_service() {
  systemctl --user disable --now keymap-overlay.service 2>/dev/null || true
  rm -f "$service_path"
  systemctl --user daemon-reload
}

restart_previous_linux_service() {
  systemctl --user daemon-reload &&
    systemctl --user enable keymap-overlay.service &&
    systemctl --user restart keymap-overlay.service
}

stop_and_remove_service() {
  "$service_uninstaller"
}

print_installed_files() {
  echo 'Installed:'
  echo "  binary: ${BINARY_PATH}"
  echo "  license: ${LICENSE_PATH}"
  echo "  third-party licenses: ${THIRD_PARTY_LICENSES_PATH}"
  echo "  installer: ${INSTALLER_PATH}"
  echo "  autostart: ${service_path}"
  echo "Using existing layer assets: ${ASSET_DIRECTORY}"
  echo "Logs: ${LOG_DIRECTORY}"
  echo "Verified release: ${release_tag}"
}

main "$@"

#!/usr/bin/env sh
# Installs or removes a verified keymap-overlay release on macOS or Linux.
set -eu

REPOSITORY='sunaemon/keymap-overlay'
# The installer's self-copy is bookkeeping used by the documented uninstall
# command, so it stays under .config.
STATE_DIRECTORY="${HOME}/.config/keymap-overlay"
LEGACY_CACHE_DIRECTORY="${HOME}/.cache/keymap-overlay"
BIN_DIRECTORY="${HOME}/.local/bin"
BINARY_PATH="${BIN_DIRECTORY}/keymap-overlay"
GENERATOR_PATH="${BIN_DIRECTORY}/keymap-overlay-generator"
GENERATOR_LICENSE_PATH="${STATE_DIRECTORY}/GENERATOR-THIRD-PARTY-LICENSES.html"
QT_BINARY_PATH="${BIN_DIRECTORY}/keymap-overlay-qt"
INSTALLER_PATH="${STATE_DIRECTORY}/install.sh"
LOG_DIRECTORY="${HOME}/.local/var/log/keymap-overlay"
QT_SERVICE_PATH="${HOME}/.config/systemd/user/keymap-overlay-qt.service"
GNOME_EXTENSION_UUID='keymap-overlay@sunaemon'
GNOME_EXTENSION_PATH="${HOME}/.local/share/gnome-shell/extensions/${GNOME_EXTENSION_UUID}"
MACOS_SERVICE_LABEL='com.sunaemon.keymap-overlay'

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

  temporary_directory="$(mktemp -d)"
  trap 'rm -rf "$temporary_directory"' EXIT HUP INT TERM
  stage_release
  mkdir -p "$STATE_DIRECTORY" "$BIN_DIRECTORY" "$LOG_DIRECTORY"
  backup_installation
  stop_service

  if install_staged_files && install_service; then
    if ! rm -rf "$LEGACY_CACHE_DIRECTORY"; then
      echo "WARNING: could not remove legacy model cache: ${LEGACY_CACHE_DIRECTORY}" >&2
    fi
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
  "$platform_file_uninstaller"
  rm -f "$BINARY_PATH" "$GENERATOR_PATH" "$GENERATOR_LICENSE_PATH" "$INSTALLER_PATH"
  rm -rf "$LEGACY_CACHE_DIRECTORY"
  echo 'Removed:'
  echo "  binary: ${BINARY_PATH}"
  echo "  generator: ${GENERATOR_PATH}"
  echo "  generator licenses: ${GENERATOR_LICENSE_PATH}"
  echo "  installer: ${INSTALLER_PATH}"
  echo "  autostart: ${service_path}"
  "$platform_file_printer"
  echo "Kept logs: ${LOG_DIRECTORY}"
}

configure_platform() {
  case "$(uname -s):$(uname -m)" in
    Darwin:arm64)
      asset_name='keymap-overlay-macos-arm64.tar.gz'
      checksum_command='shasum'
      service_path="${HOME}/Library/LaunchAgents/${MACOS_SERVICE_LABEL}.plist"
      service_installer=install_macos_service
      service_stopper=stop_macos_service
      service_uninstaller=uninstall_macos_service
      previous_service_restarter=restart_previous_macos_service
      platform_staged_validator=validate_no_extra_staged_files
      platform_file_backupper=backup_macos_files
      platform_file_installer=install_no_extra_files
      platform_file_restorer=restore_no_extra_files
      platform_file_uninstaller=uninstall_no_extra_files
      platform_file_printer=print_no_extra_files
      ;;
    Linux:x86_64|Linux:amd64)
      asset_name='keymap-overlay-linux-x86_64.tar.gz'
      checksum_command='sha256sum'
      service_path="${HOME}/.config/systemd/user/keymap-overlay.service"
      service_installer=install_linux_service
      service_stopper=stop_linux_service
      service_uninstaller=uninstall_linux_service
      previous_service_restarter=restart_previous_linux_service
      platform_staged_validator=validate_linux_staged_files
      platform_file_backupper=backup_linux_files
      platform_file_installer=install_linux_files
      platform_file_restorer=restore_linux_files
      platform_file_uninstaller=uninstall_linux_files
      platform_file_printer=print_linux_files
      ;;
    Linux:aarch64|Linux:arm64)
      asset_name='keymap-overlay-linux-arm64.tar.gz'
      checksum_command='sha256sum'
      service_path="${HOME}/.config/systemd/user/keymap-overlay.service"
      service_installer=install_linux_service
      service_stopper=stop_linux_service
      service_uninstaller=uninstall_linux_service
      previous_service_restarter=restart_previous_linux_service
      platform_staged_validator=validate_linux_staged_files
      platform_file_backupper=backup_linux_files
      platform_file_installer=install_linux_files
      platform_file_restorer=restore_linux_files
      platform_file_uninstaller=uninstall_linux_files
      platform_file_printer=print_linux_files
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
  "$platform_staged_validator"
}

validate_no_extra_staged_files() {
  :
}

validate_linux_staged_files() {
  for file in \
    keymap-overlay-qt \
    gnome-shell/keymap-overlay@sunaemon/metadata.json \
    gnome-shell/keymap-overlay@sunaemon/extension.js \
    gnome-shell/keymap-overlay@sunaemon/stylesheet.css; do
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
  backup_file "$GENERATOR_PATH" generator
  backup_file "$GENERATOR_LICENSE_PATH" generator-license
  backup_file "$INSTALLER_PATH" installer
  backup_file "$service_path" service
  "$platform_file_backupper"
}

backup_no_extra_files() {
  :
}

backup_macos_files() {
  if [ -f "$service_path" ] &&
    launchctl print "gui/$(id -u)/${MACOS_SERVICE_LABEL}" >/dev/null 2>&1; then
    : >"${temporary_directory}/was-service-loaded"
  fi
}

backup_linux_files() {
  backup_file "$QT_BINARY_PATH" qt-binary
  backup_file "$QT_SERVICE_PATH" qt-service
  backup_directory "$GNOME_EXTENSION_PATH" gnome-extension
  backup_linux_unit_state "$service_path" keymap-overlay.service service
  backup_linux_unit_state "$QT_SERVICE_PATH" keymap-overlay-qt.service qt-service
}

backup_linux_unit_state() {
  unit_path=$1
  unit_name=$2
  backup_name=$3
  if [ ! -f "$unit_path" ]; then
    return
  fi
  if systemctl --user is-enabled --quiet "$unit_name"; then
    : >"${temporary_directory}/was-${backup_name}-enabled"
  fi
  if systemctl --user is-active --quiet "$unit_name"; then
    : >"${temporary_directory}/was-${backup_name}-active"
  fi
}

backup_file() {
  source_path=$1
  backup_name=$2
  if [ -f "$source_path" ]; then
    cp -p "$source_path" "${temporary_directory}/backup-${backup_name}"
    : >"${temporary_directory}/had-${backup_name}"
  fi
}

backup_directory() {
  source_path=$1
  backup_name=$2
  if [ -d "$source_path" ]; then
    cp -pR "$source_path" "${temporary_directory}/backup-${backup_name}"
    : >"${temporary_directory}/had-${backup_name}"
  fi
}

stop_service() {
  "$service_stopper"
}

# The archive still carries LICENSE and THIRD-PARTY-LICENSES.html for anyone
# packaging this where a distribution requires them as files; the binary embeds
# both, so nothing is installed for them here.
install_staged_files() {
  install -m 755 "${temporary_directory}/keymap-overlay" "$BINARY_PATH" &&
    rm -f "$GENERATOR_PATH" "$GENERATOR_LICENSE_PATH" &&
    install -m 755 "$staged_installer" "$INSTALLER_PATH" &&
    "$platform_file_installer"
}

install_no_extra_files() {
  :
}

install_linux_files() {
  install -m 755 "${temporary_directory}/keymap-overlay-qt" "$QT_BINARY_PATH" || return
  mkdir -p "$GNOME_EXTENSION_PATH" || return
  install -m 644 "${temporary_directory}/gnome-shell/${GNOME_EXTENSION_UUID}/metadata.json" \
    "${GNOME_EXTENSION_PATH}/metadata.json" &&
    install -m 644 "${temporary_directory}/gnome-shell/${GNOME_EXTENSION_UUID}/extension.js" \
      "${GNOME_EXTENSION_PATH}/extension.js" &&
    install -m 644 "${temporary_directory}/gnome-shell/${GNOME_EXTENSION_UUID}/stylesheet.css" \
      "${GNOME_EXTENSION_PATH}/stylesheet.css"
}

install_service() {
  "$service_installer"
}

restore_installation() {
  restore_file "$BINARY_PATH" binary
  restore_file "$GENERATOR_PATH" generator
  restore_file "$GENERATOR_LICENSE_PATH" generator-license
  restore_file "$INSTALLER_PATH" installer
  restore_file "$service_path" service
  "$platform_file_restorer"
}

restore_no_extra_files() {
  :
}

restore_linux_files() {
  restore_file "$QT_BINARY_PATH" qt-binary
  restore_file "$QT_SERVICE_PATH" qt-service
  restore_directory "$GNOME_EXTENSION_PATH" gnome-extension
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

restore_directory() {
  destination=$1
  backup_name=$2
  rm -rf "$destination"
  if [ -f "${temporary_directory}/had-${backup_name}" ]; then
    mkdir -p "$(dirname "$destination")"
    cp -pR "${temporary_directory}/backup-${backup_name}" "$destination"
  fi
}

restart_previous_service() {
  if [ -f "${temporary_directory}/had-service" ]; then
    "$previous_service_restarter" || true
  fi
}

xml_escape() {
  printf '%s\n' "$1" | awk '{
    gsub(/&/, "\\&amp;")
    gsub(/</, "\\&lt;")
    gsub(/>/, "\\&gt;")
    print
  }'
}

# launchd never rotates what it redirects, so the overlay owns its own log file
# here.
install_macos_service() {
  binary_xml="$(xml_escape "$BINARY_PATH")"
  log_xml="$(xml_escape "${LOG_DIRECTORY}/overlay.log")"
  mkdir -p "$(dirname "$service_path")" || return
  cat >"${service_path}.tmp" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>${MACOS_SERVICE_LABEL}</string>
  <key>ProgramArguments</key>
  <array>
    <string>${binary_xml}</string>
    <string>--log-out</string>
    <string>${log_xml}</string>
  </array>
  <key>RunAtLoad</key>
  <true/>
  <key>KeepAlive</key>
  <dict><key>SuccessfulExit</key><false/></dict>
  <key>ProcessType</key>
  <string>Interactive</string>
</dict>
</plist>
EOF
  mv "${service_path}.tmp" "$service_path" || return
  bootstrap_macos_service
}

bootstrap_macos_service() {
  attempt=1
  while ! output="$(launchctl bootstrap "gui/$(id -u)" "$service_path" 2>&1)"; do
    if [ "$attempt" -ge 3 ]; then
      printf '%s\n' "$output" >&2
      return 1
    fi
    printf '%s\n' 'launchctl bootstrap failed; retrying...' >&2
    attempt=$((attempt + 1))
    sleep 1
  done
}

stop_macos_service() {
  launchctl bootout "gui/$(id -u)/${MACOS_SERVICE_LABEL}" 2>/dev/null || true
}

uninstall_macos_service() {
  stop_macos_service
  rm -f "$service_path"
}

restart_previous_macos_service() {
  if [ -f "${temporary_directory}/was-service-loaded" ]; then
    bootstrap_macos_service
  fi
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
ExecStart="${BINARY_PATH}"
# The log is left on stderr for journald, which timestamps, rotates and retains
# it: journalctl --user -u keymap-overlay
SyslogIdentifier=keymap-overlay
Restart=on-failure
RestartSec=2

[Install]
WantedBy=graphical-session.target
EOF
  mv "${service_path}.tmp" "$service_path" || return
  cat >"${QT_SERVICE_PATH}.tmp" <<EOF
[Unit]
Description=QMK keymap layer Qt renderer
Documentation=https://github.com/${REPOSITORY}
PartOf=graphical-session.target
After=graphical-session.target keymap-overlay.service
Wants=keymap-overlay.service
StartLimitIntervalSec=0

[Service]
Type=simple
ExecStart="${QT_BINARY_PATH}"
Restart=on-failure
RestartSec=2

[Install]
WantedBy=graphical-session.target
EOF
  mv "${QT_SERVICE_PATH}.tmp" "$QT_SERVICE_PATH" || return
  systemctl --user daemon-reload &&
    systemctl --user enable keymap-overlay.service &&
    systemctl --user restart keymap-overlay.service || return
  if [ -n "${KEYMAP_OVERLAY_FORCE_QT:-}" ] ||
    ! printf '%s' "${XDG_CURRENT_DESKTOP:-}" | grep -Eqi '(^|:)gnome(:|$)'; then
    systemctl --user enable keymap-overlay-qt.service &&
      systemctl --user restart keymap-overlay-qt.service || return
  else
    systemctl --user disable --now keymap-overlay-qt.service || return
  fi
  if command -v gnome-extensions >/dev/null 2>&1 &&
    printf '%s' "${XDG_CURRENT_DESKTOP:-}" | grep -qi gnome; then
    gnome-extensions enable "$GNOME_EXTENSION_UUID" ||
      echo "NOTE: log out and back in, then enable ${GNOME_EXTENSION_UUID}."
  fi
}

stop_linux_service() {
  systemctl --user stop keymap-overlay-qt.service 2>/dev/null || true
  systemctl --user stop keymap-overlay.service 2>/dev/null || true
}

uninstall_linux_service() {
  systemctl --user disable --now keymap-overlay-qt.service 2>/dev/null || true
  systemctl --user disable --now keymap-overlay.service 2>/dev/null || true
  rm -f "$service_path"
  rm -f "$QT_SERVICE_PATH"
  systemctl --user daemon-reload
}

restart_previous_linux_service() {
  systemctl --user daemon-reload || return
  restore_linux_unit_state keymap-overlay.service service || return
  if [ -f "$QT_SERVICE_PATH" ]; then
    restore_linux_unit_state keymap-overlay-qt.service qt-service
  fi
}

restore_linux_unit_state() {
  unit_name=$1
  backup_name=$2
  if [ -f "${temporary_directory}/was-${backup_name}-enabled" ]; then
    systemctl --user enable "$unit_name" || return
  else
    systemctl --user disable "$unit_name" || return
  fi
  if [ -f "${temporary_directory}/was-${backup_name}-active" ]; then
    systemctl --user restart "$unit_name"
  else
    systemctl --user stop "$unit_name"
  fi
}

stop_and_remove_service() {
  "$service_uninstaller"
}

uninstall_no_extra_files() {
  :
}

uninstall_linux_files() {
  rm -f "$QT_BINARY_PATH"
  rm -rf "$GNOME_EXTENSION_PATH"
}

print_no_extra_files() {
  :
}

print_linux_files() {
  echo "  Qt renderer: ${QT_BINARY_PATH}"
  echo "  GNOME extension: ${GNOME_EXTENSION_PATH}"
  echo "  Qt autostart: ${QT_SERVICE_PATH}"
}

print_installed_files() {
  echo 'Installed:'
  echo "  binary: ${BINARY_PATH}"
  echo "  installer: ${INSTALLER_PATH}"
  echo "  autostart: ${service_path}"
  "$platform_file_printer"
  echo "Logs: ${LOG_DIRECTORY}"
  echo "Licenses: ${BINARY_PATH} --license, --third-party-licenses"
  echo "Verified release: ${release_tag}"
  warn_if_not_on_path
}

# The service definitions name the binary by absolute path, so this only affects
# running it by hand.
warn_if_not_on_path() {
  case ":${PATH}:" in
    *":${BIN_DIRECTORY}:"*) ;;
    *)
      echo "NOTE: ${BIN_DIRECTORY} is not on PATH; the login service is unaffected." >&2
      ;;
  esac
}

main "$@"

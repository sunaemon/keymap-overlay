# Connected keyboards provide their models directly from Vial at startup, so
# installation does not generate, copy, or persist layer models.
.PHONY: install-overlay
install-overlay: build-overlay
	@mkdir -p "$(KEYMAP_OVERLAY_BIN_DIR)" "$(KEYMAP_OVERLAY_LOG_DIR)"
# Windows holds an open executable locked, so the running overlay has to go
# before its binary can be replaced. The other two systems replace the file
# underneath the running process and stop it as part of installing the service.
	@$(MAKE) _stop_service_$(OS_FAMILY)
	install -C "$(OVERLAY_BUILD_BINARY)" "$(KEYMAP_OVERLAY_BINARY)"
	@$(MAKE) _install_renderer_$(OS_FAMILY)
	@$(MAKE) _install_service_$(OS_FAMILY)
	@echo "✔ Overlay installed and started; logs: $(KEYMAP_OVERLAY_LOG_DIR)"

# Nothing to do where a running binary can be replaced in place; the service is
# stopped and started again by _install_service_<system> below. The `:` keeps
# make from reporting that there was nothing to do on every install.
.PHONY: _stop_service_macos
_stop_service_macos:
	@:

.PHONY: _stop_service_linux
_stop_service_linux:
	@:

.PHONY: _stop_service_windows
_stop_service_windows:
	@$(STOP_KEYMAP_OVERLAY_PROCESS)

.PHONY: _install_renderer_macos
_install_renderer_macos:
	@:

.PHONY: _install_renderer_linux
_install_renderer_linux:
	install -C "target/release/keymap-overlay-qt" "$(KEYMAP_OVERLAY_QT_BINARY)"
	@mkdir -p "$(GNOME_EXTENSION_DIR)"
	install -m 0644 "$(GNOME_EXTENSION_SOURCE)/metadata.json" "$(GNOME_EXTENSION_DIR)/metadata.json"
	install -m 0644 "$(GNOME_EXTENSION_SOURCE)/extension.js" "$(GNOME_EXTENSION_DIR)/extension.js"
	install -m 0644 "$(GNOME_EXTENSION_SOURCE)/stylesheet.css" "$(GNOME_EXTENSION_DIR)/stylesheet.css"

.PHONY: _install_renderer_windows
_install_renderer_windows:
	@:

# launchd never rotates what it redirects, so the overlay owns its own log file
# here. Both paths are arguments because the Windows Run key carries arguments
# and no environment at all.
.PHONY: _install_service_macos
_install_service_macos:
	@mkdir -p "$(dir $(KEYMAP_OVERLAY_PLIST))"
	@{ \
		printf '%s\n' \
		'<?xml version="1.0" encoding="UTF-8"?>' \
		'<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">' \
		'<plist version="1.0">' \
		'<dict>' \
		'  <key>Label</key>' \
		'  <string>$(KEYMAP_OVERLAY_LABEL)</string>' \
		'  <key>ProgramArguments</key>' \
		'  <array>' \
		'    <string>$(call xml_escape,$(KEYMAP_OVERLAY_BINARY))</string>' \
		'    <string>--log-out</string>' \
		'    <string>$(call xml_escape,$(KEYMAP_OVERLAY_LOG_FILE))</string>' \
		'  </array>' \
		'  <key>RunAtLoad</key>' \
		'  <true/>' \
		'  <key>KeepAlive</key>' \
		'  <dict><key>SuccessfulExit</key><false/></dict>' \
		'  <key>ProcessType</key>' \
		'  <string>Interactive</string>' \
		'</dict>' \
		'</plist>'; \
		} > "$(KEYMAP_OVERLAY_PLIST).tmp" && mv "$(KEYMAP_OVERLAY_PLIST).tmp" "$(KEYMAP_OVERLAY_PLIST)"
	@$(BOOTOUT_KEYMAP_OVERLAY)
	@$(BOOTSTRAP_KEYMAP_OVERLAY)

# Paths are quoted so that a directory containing spaces still parses.
.PHONY: _install_service_linux
_install_service_linux:
	@mkdir -p "$(dir $(KEYMAP_OVERLAY_UNIT))"
	@{ \
		printf '%s\n' \
		'[Unit]' \
		'Description=QMK keymap layer overlay' \
		'Documentation=https://github.com/sunaemon/keymap-overlay' \
		'# The daemon owns HID and publishes renderer state over session D-Bus.' \
		'PartOf=graphical-session.target' \
		'After=graphical-session.target' \
		'# The compositor may still be coming up when the session target is' \
		'# reached, and each retry is a start; without this the rate limiter' \
		'# would give up before it is listening.' \
		'StartLimitIntervalSec=0' \
		'' \
		'[Service]' \
		'Type=simple' \
		'ExecStart="$(KEYMAP_OVERLAY_BINARY)"' \
		'# The log is left on stderr for journald, which timestamps, rotates' \
		'# and retains it: journalctl --user -u keymap-overlay' \
		'SyslogIdentifier=keymap-overlay' \
		'# Matches KeepAlive/SuccessfulExit=false in the launchd plist:' \
		'# come back after a crash, stay stopped after a clean exit.' \
		'Restart=on-failure' \
		'RestartSec=2' \
		'' \
		'[Install]' \
		'WantedBy=graphical-session.target'; \
		} > "$(KEYMAP_OVERLAY_UNIT).tmp" && mv "$(KEYMAP_OVERLAY_UNIT).tmp" "$(KEYMAP_OVERLAY_UNIT)"
	@{ \
		printf '%s\n' \
		'[Unit]' \
		'Description=QMK keymap layer Qt renderer' \
		'Documentation=https://github.com/sunaemon/keymap-overlay' \
		'PartOf=graphical-session.target' \
		'After=graphical-session.target $(KEYMAP_OVERLAY_UNIT_NAME)' \
		'Wants=$(KEYMAP_OVERLAY_UNIT_NAME)' \
		'StartLimitIntervalSec=0' \
		'' \
		'[Service]' \
		'Type=simple' \
		'ExecStart="$(KEYMAP_OVERLAY_QT_BINARY)"' \
		'Restart=on-failure' \
		'RestartSec=2' \
		'' \
		'[Install]' \
		'WantedBy=graphical-session.target'; \
		} > "$(KEYMAP_OVERLAY_QT_UNIT).tmp" && mv "$(KEYMAP_OVERLAY_QT_UNIT).tmp" "$(KEYMAP_OVERLAY_QT_UNIT)"
	systemctl --user daemon-reload
	systemctl --user enable "$(KEYMAP_OVERLAY_UNIT_NAME)"
# restart, not start: this is also the update path, and the running process
# still holds the previous binary.
	systemctl --user restart "$(KEYMAP_OVERLAY_UNIT_NAME)"
	@if [ -n "$${KEYMAP_OVERLAY_FORCE_QT:-}" ] || ! printf '%s' "$${XDG_CURRENT_DESKTOP:-}" | grep -Eqi '(^|:)gnome(:|$$)'; then \
		systemctl --user enable "$(KEYMAP_OVERLAY_QT_UNIT_NAME)"; \
		systemctl --user restart "$(KEYMAP_OVERLAY_QT_UNIT_NAME)"; \
	else \
		systemctl --user disable --now "$(KEYMAP_OVERLAY_QT_UNIT_NAME)"; \
	fi
	@if command -v gnome-extensions >/dev/null 2>&1 && printf '%s' "$${XDG_CURRENT_DESKTOP:-}" | grep -qi gnome; then \
		gnome-extensions enable "$(GNOME_EXTENSION_UUID)" || \
			echo "NOTE: log out and back in, then enable $(GNOME_EXTENSION_UUID)."; \
	fi

# The current user's Run key starts the overlay at sign-in without requiring an
# administrator to create a Task Scheduler entry.
#
# The native Rust frontend accepts the same log argument as the other
# platforms, so Windows does not need a C ABI or a special logging path.
.PHONY: _install_service_windows
_install_service_windows:
# set -e so a failing cygpath does not hand an empty path to the registry, and
# $ErrorActionPreference so PowerShell's non-terminating errors become failures
# make can see: without it, Set-ItemProperty or Start-Process can fail while
# powershell.exe still exits 0 and install-overlay reports success.
	@set -e; \
	binary="$$(cygpath -w "$(KEYMAP_OVERLAY_BINARY)")"; \
	log_file="$$(cygpath -w "$(KEYMAP_OVERLAY_LOG_FILE)")"; \
	env KEYMAP_OVERLAY_BINARY="$$binary" KEYMAP_OVERLAY_LOG_FILE="$$log_file" MSYS2_ARG_CONV_EXCL='*' powershell.exe -NoProfile -NonInteractive -Command \
	'$$ErrorActionPreference = "Stop"; $$quote = [char]34; $$command = $$quote + $$env:KEYMAP_OVERLAY_BINARY + $$quote + " --log-out " + $$quote + $$env:KEYMAP_OVERLAY_LOG_FILE + $$quote; Set-ItemProperty -Path "HKCU:\Software\Microsoft\Windows\CurrentVersion\Run" -Name "$(KEYMAP_OVERLAY_RUN_VALUE)" -Value $$command; Start-Process -FilePath $$env:KEYMAP_OVERLAY_BINARY -ArgumentList @("--log-out", $$env:KEYMAP_OVERLAY_LOG_FILE)'

.PHONY: uninstall-overlay
uninstall-overlay:
	@$(MAKE) _uninstall_service_$(OS_FAMILY)
	rm -f "$(KEYMAP_OVERLAY_BINARY)"
	# Remove the legacy separate generator when upgrading an older install.
	rm -f "$(KEYMAP_OVERLAY_BIN_DIR)/keymap-overlay-generator$(EXE_SUFFIX)"
	@$(MAKE) _uninstall_renderer_$(OS_FAMILY)
	@$(MAKE) _remove_legacy_cache_$(OS_FAMILY)
	@echo "✔ Overlay service removed; logs remain at $(KEYMAP_OVERLAY_LOG_DIR)"

.PHONY: _remove_legacy_cache_macos
_remove_legacy_cache_macos _remove_legacy_cache_linux:
	rm -rf "$(HOME)/.cache/keymap-overlay"

.PHONY: _remove_legacy_cache_windows
_remove_legacy_cache_windows:
	rm -f "$(WINDOWS_LOCAL_APP_DATA)/keymap-overlay"/*.png
	rm -f "$(WINDOWS_LOCAL_APP_DATA)/keymap-overlay"/[0-9]*.json

.PHONY: _uninstall_service_macos
_uninstall_service_macos:
	@$(BOOTOUT_KEYMAP_OVERLAY)
	rm -f "$(KEYMAP_OVERLAY_PLIST)"

.PHONY: _uninstall_service_linux
_uninstall_service_linux:
	@$(STOP_KEYMAP_OVERLAY_QT_UNIT)
	@$(STOP_KEYMAP_OVERLAY_UNIT)
	rm -f "$(KEYMAP_OVERLAY_UNIT)"
	rm -f "$(KEYMAP_OVERLAY_QT_UNIT)"
	systemctl --user daemon-reload

.PHONY: _uninstall_service_windows
_uninstall_service_windows:
	@$(STOP_KEYMAP_OVERLAY_PROCESS)
	@MSYS2_ARG_CONV_EXCL='*' powershell.exe -NoProfile -NonInteractive -Command \
		'Remove-ItemProperty -Path "HKCU:\Software\Microsoft\Windows\CurrentVersion\Run" -Name "$(KEYMAP_OVERLAY_RUN_VALUE)" -ErrorAction SilentlyContinue; exit 0'

.PHONY: _uninstall_renderer_macos
_uninstall_renderer_macos:
	@:

.PHONY: _uninstall_renderer_linux
_uninstall_renderer_linux:
	rm -f "$(KEYMAP_OVERLAY_QT_BINARY)"
	rm -rf "$(GNOME_EXTENSION_DIR)"

.PHONY: _uninstall_renderer_windows
_uninstall_renderer_windows:
	@:

# Linux only: macOS asks for Input Monitoring permission instead, which is
# granted in System Settings rather than by a file, and Windows needs no
# permission at all to read a vendor-defined HID interface.
.PHONY: install-udev-rules
install-udev-rules:
ifneq ($(OS_FAMILY),linux)
	$(error install-udev-rules is Linux-only; macOS grants Raw HID access through Input Monitoring, and Windows needs no grant)
endif

	@mkdir -p build
	@{ \
		printf '%s\n' \
		'# Generated by "make install-udev-rules"; edits are overwritten.' \
		'#' \
		'# uaccess hands the Raw HID node to whoever is logged in at the seat,' \
		'# which is what lets the overlay read layer reports without root.'; \
		for kb in $(ALL_KEYBOARD_IDS); do \
			$(MAKE) --no-print-directory print-udev-rule KEYBOARD_ID=$$kb || exit 1; \
		done; \
		} > build/keymap-overlay.rules.tmp && mv build/keymap-overlay.rules.tmp build/keymap-overlay.rules
	$(SUDO) install -m 0644 build/keymap-overlay.rules "$(KEYMAP_OVERLAY_UDEV_RULES)"
	$(SUDO) udevadm control --reload
	$(SUDO) udevadm trigger --subsystem-match=hidraw
	@echo "✔ Rules installed at $(KEYMAP_OVERLAY_UDEV_RULES); replug a keyboard if the overlay still cannot open it"

.PHONY: install-uhid-test-rule
install-uhid-test-rule:
ifeq ($(OS_FAMILY),linux)
	$(SUDO) install -m 0644 \
		overlay/platforms/linux/tests/99-keymap-overlay-e2e.rules \
		/etc/udev/rules.d/99-keymap-overlay-e2e.rules
	$(SUDO) udevadm control --reload
	@echo "✔ Virtual HID test rule installed; existing devices need to be recreated."
else
	$(error install-uhid-test-rule is only available on Linux)
endif

.PHONY: uninstall-uhid-test-rule
uninstall-uhid-test-rule:
ifeq ($(OS_FAMILY),linux)
	$(SUDO) rm -f /etc/udev/rules.d/99-keymap-overlay-e2e.rules
	$(SUDO) udevadm control --reload
	@echo "✔ Virtual HID test rule removed."
else
	$(error uninstall-uhid-test-rule is only available on Linux)
endif

.PHONY: uninstall-udev-rules
uninstall-udev-rules:
ifneq ($(OS_FAMILY),linux)
	$(error uninstall-udev-rules is Linux-only)
endif
	$(SUDO) rm -f "$(KEYMAP_OVERLAY_UDEV_RULES)"
	$(SUDO) udevadm control --reload
	@echo "✔ Rules removed from $(KEYMAP_OVERLAY_UDEV_RULES)"

.PHONY: print-udev-rule
print-udev-rule:
ifndef KEYBOARD_ID
	$(error KEYBOARD_ID is required for print-udev-rule)
endif
	@vid="$$( $(UV) run python -m model.scripts.get_keyboard_metadata "$(KEYBOARD_JSON)" vid)" || exit 1; \
		pid="$$( $(UV) run python -m model.scripts.get_keyboard_metadata "$(KEYBOARD_JSON)" pid)" || exit 1; \
		printf '\n# %s (KEYBOARD_ID=%s)\nKERNEL=="hidraw*", SUBSYSTEM=="hidraw", ATTRS{idVendor}=="%s", ATTRS{idProduct}=="%s", TAG+="uaccess"\n' \
		"$(QMK_KEYBOARD)" "$(KEYBOARD_ID)" "$$vid" "$$pid"

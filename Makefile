SHELL := /bin/bash

# ================= PLATFORM CONFIGURATION =================

# The image generation workflow is portable; the overlay's window, its login
# service, the toolchain packages, and the firmware workflow are not. Every
# target that differs between the systems dispatches on this.
#
# Windows means an MSYS2 or Git Bash shell driving a native Windows build:
# `uname -s` there reports MINGW64_NT-10.0-… or MSYS_NT-…, which no `ifeq` can
# match exactly, hence findstring. Compiling and flashing firmware is not
# supported on it — see `_setup_toolchain_windows`.
UNAME_S := $(shell uname -s)
ifeq ($(UNAME_S),Darwin)
OS_FAMILY := macos
else ifeq ($(UNAME_S),Linux)
OS_FAMILY := linux
else ifneq (,$(findstring MINGW,$(UNAME_S)))
OS_FAMILY := windows
else ifneq (,$(findstring MSYS,$(UNAME_S)))
OS_FAMILY := windows
else
$(error keymap-overlay supports macOS, Linux and Windows, not '$(UNAME_S)')
endif

# Cargo names the binary after the target, and the login service needs the name
# that exists on disk.
ifeq ($(OS_FAMILY),windows)
EXE_SUFFIX := .exe
else
EXE_SUFFIX :=
endif

# QMK's Windows toolchain is QMK MSYS, a separate environment from the one that
# builds the overlay, and the boards here flash over USB or a mounted UF2
# volume that MSYS2 cannot reach either. Rather than half-support it, the
# firmware targets say where to go.
WINDOWS_FIRMWARE_ERROR := is not supported on Windows; compile and flash from WSL, macOS or Linux (see Platform Support in README.md)

# ================= VIA CONFIGURATION =================

# If VIAL is enabled, the keymap will load from the VIAL EEPROM dump in `make install` and `make draw-layers`.
# If VIAL is disabled, the keymap will be compiled from the firmware source.
VIAL ?= false

# ================= TOOLS CONFIGURATION =================
RSVG ?= $(MISE) exec -- resvg
MISE ?= mise
# Format and lint tools are pinned in mise.dev.toml, which mise only loads when
# MISE_ENV=dev is set.
MISE_DEV ?= MISE_ENV=dev $(MISE)
BREW ?= brew
ifeq ($(OS_FAMILY),macos)
# The osx-cross compilers are keg-only, so their paths are prepended for the
# QMK commands rather than left to the shell profile.
QMK_TOOLCHAIN_PATH = $(shell $(BREW) --prefix arm-none-eabi-gcc@8 2>/dev/null)/bin:$(shell $(BREW) --prefix arm-none-eabi-binutils 2>/dev/null)/bin
QMK_ENV = PATH="$(QMK_TOOLCHAIN_PATH):$$PATH"
else
# Linux distributions put the ARM and AVR compilers on PATH themselves.
QMK_ENV =
endif
VITALY_VERSION := $(shell awk -F' *= *' '/^VITALY_VERSION *=/ {gsub(/"/,"",$$2); print $$2}' mise.toml)
ifeq ($(strip $(VITALY_VERSION)),)
$(error VITALY_VERSION is missing from mise.toml)
endif
# Only the Linux targets that write outside $HOME use this: installing
# distribution packages, the udev rules, and mounting the UF2 bootloader
# volume while flashing.
SUDO ?= sudo
# The volume an rp2040 bootloader exposes. Override it for a board whose
# bootloader labels its volume differently.
UF2_VOLUME_LABEL ?= RPI-RP2
CARGO_AUDIT ?= $(MISE_DEV) exec -- cargo-audit
LEFTHOOK ?= $(MISE) exec -- lefthook
QMK ?= $(QMK_ENV) $(MISE) exec -- qmk
KEYMAP ?= $(MISE) exec -- keymap
UV ?= $(MISE) exec -- uv
CARGO ?= $(MISE) exec -- cargo
VITALY ?= $(MISE) exec cargo:vitaly@$(VITALY_VERSION) -- vitaly

QMK_TOOLCHAIN_PACKAGES := osx-cross/arm/arm-none-eabi-gcc@8 osx-cross/avr/avr-gcc@9 avrdude dfu-programmer dfu-util

# The same set per distribution, plus the libraries the overlay itself links:
# libudev for hidraw enumeration, libwayland-client for the layer-shell window,
# and libX11 for the fallback one.
LINUX_TOOLCHAIN_PACKAGES_PACMAN := arm-none-eabi-gcc arm-none-eabi-binutils arm-none-eabi-newlib avr-gcc avr-libc avrdude dfu-programmer dfu-util systemd-libs wayland libx11
LINUX_TOOLCHAIN_PACKAGES_APT := gcc-arm-none-eabi binutils-arm-none-eabi libnewlib-arm-none-eabi gcc-avr avr-libc avrdude dfu-programmer dfu-util libudev-dev libwayland-dev libx11-dev
LINUX_TOOLCHAIN_PACKAGES_DNF := arm-none-eabi-gcc-cs arm-none-eabi-newlib avr-gcc avr-libc avrdude dfu-programmer dfu-util systemd-devel wayland-devel libX11-devel

# Escape XML character data so that a HOME containing & or < still produces a
# valid plist. Ampersands must be substituted first.
xml_escape = $(subst >,&gt;,$(subst <,&lt;,$(subst &,&amp;,$(1))))

# Run a command and atomically replace its output, removing partial output on failure.
define WRITE_OUTPUT
trap 'rm -f "$(1).tmp"' EXIT; \
$(2) > "$(1).tmp" && mv "$(1).tmp" "$(1)" && trap - EXIT
endef

# Stop the login service, tolerating only the expected case where it is not loaded.
define BOOTOUT_KEYMAP_OVERLAY
if output="$$(launchctl bootout "gui/$$(id -u)/$(KEYMAP_OVERLAY_LABEL)" 2>&1)"; then \
	:; \
	else \
	case "$$output" in \
		""|*"Could not find service"*|*"No such process"*) ;; \
		*) printf '%s\n' "$$output" >&2; exit 1 ;; \
	esac; \
	fi
endef

# The systemd counterpart. The unit file is ours, so its absence is what "not
# installed" means; systemctl would fail on a unit it has never seen.
define STOP_KEYMAP_OVERLAY_UNIT
if [ -f "$(KEYMAP_OVERLAY_UNIT)" ]; then \
	systemctl --user disable --now "$(KEYMAP_OVERLAY_UNIT_NAME)"; \
	fi
endef

# The Task Scheduler counterpart. -ErrorAction SilentlyContinue covers the task
# never having been registered, which is the only failure worth tolerating; the
# command is single-quoted so that the shell leaves PowerShell's $ alone, and
# MSYS2_ARG_CONV_EXCL stops MSYS2 rewriting the arguments as paths.
#
# Run through `env` so the line does not open with NAME=VALUE, which the
# Makefile formatter rewrites to NAME = VALUE — turning the variable this needs
# in the environment into a command it would try to run.
define STOP_KEYMAP_OVERLAY_TASK
env MSYS2_ARG_CONV_EXCL='*' powershell.exe -NoProfile -NonInteractive -Command \
	'Stop-ScheduledTask -TaskName "$(KEYMAP_OVERLAY_TASK_NAME)" -ErrorAction SilentlyContinue'
endef

# ================= QMK CONFIGURATION =================
QMK_HOME := qmk_firmware
export QMK_HOME := $(QMK_HOME)

QMK_KEYMAP ?= keymap

# Directory containing keyboard configurations
KEYBOARDS_DIR ?= example

ifdef KEYBOARD_ID

# KEYBOARD_ID names a directory in $(KEYBOARDS_DIR), is compiled into the
# firmware as -DKEYBOARD_ID, and travels in one byte of the Raw HID report,
# so it has to be an integer that fits in a uint8_t.
ifneq ($(shell printf '%s' "$(KEYBOARD_ID)" | grep -Eq '^([0-9]|[1-9][0-9]|1[0-9][0-9]|2[0-4][0-9]|25[0-5])$$' && echo ok),ok)
    $(error KEYBOARD_ID must be an integer between 0 and 255, got '$(KEYBOARD_ID)')
endif

# QMK keyboard name (e.g., salicylic_acid3/insixty_en).
# Read from $(KEYBOARDS_DIR)/$(KEYBOARD_ID)/config.json.
QMK_KEYBOARD ?= $(shell awk -F'"' '/qmk_keyboard/ {print $$4}' $(KEYBOARDS_DIR)/$(KEYBOARD_ID)/config.json)
ifeq ($(QMK_KEYBOARD),)
    $(error KEYBOARD_ID=$(KEYBOARD_ID) is not valid or $(KEYBOARDS_DIR)/$(KEYBOARD_ID)/config.json is missing or malformed)
endif
QMK_FLAGS += -e KEYBOARD_ID=$(KEYBOARD_ID)

KEYMAP_PREFIX := $(KEYBOARD_ID)_

# [QMK Keyboard JSON]
# QMK keyboard definition (matrix/layouts/metadata).
# Type: src/types.py:KeyboardJson
KEYBOARD_JSON := $(KEYBOARDS_DIR)/$(KEYBOARD_ID)/keyboard.json
QMK_KEYMAP_C := $(KEYBOARDS_DIR)/$(KEYBOARD_ID)/keymap/keymap.c

# Evaluated on first use and then cached, so that targets which never need it
# (clean, compile, flash, print-vars) do not pay for a Python startup.
DEVICE_PID = $(eval DEVICE_PID := $(shell $(UV) run python -c "import json; print(int(json.load(open('$(KEYBOARD_JSON)'))['usb']['pid'], 16))"))$(DEVICE_PID)

LAYOUT_NAME := LAYOUT

DPI ?= 144

# ================= BUILD CONFIGURATION =================
BUILD_DIR := build/$(KEYBOARD_ID)
ABS_BUILD_DIR := $(abspath $(BUILD_DIR))

QMK_FLAGS += -e BUILD_DIR=$(ABS_BUILD_DIR)/qmk_build

# [QMK Keymap JSON]
# Contains the full keymap definition (layers, keycodes) in QMK format.
# Type: src/types.py:QmkKeymapJson
# Generated from: 'qmk c2json' (source) or 'generate_qmk_keymap_from_vitaly.py' (VIAL).
# Used by: 'keymap-drawer' (visuals), 'generate_vitaly_layout.py' (flashing).
QMK_KEYMAP_JSON := $(BUILD_DIR)/qmk-keymap.json

# [Raw QMK Keymap JSON]
# Unprocessed QMK JSON used as input for postprocessing.
QMK_KEYMAP_JSON_RAW := $(BUILD_DIR)/qmk-keymap.raw.json

# [Keymap Drawer YAML]
# Intermediate representation for keymap-drawer.
# Type: keymap-drawer schema (not in src/types.py)
# Generated from: 'keymap parse' using $(QMK_KEYMAP_JSON).
# Used by: 'keymap draw' to generate SVG images.
KEYMAP_DRAWER_YAML := $(BUILD_DIR)/keymap-drawer.yaml

# [Keycodes JSON]
# Mapping of QMK hex keycodes to their string names (e.g., 0x0004 -> KC_A).
# Type: src/types.py:KeycodesJson
# Generated from: 'generate_keycodes.py' scanning QMK firmware.
# Used by: 'postprocess_qmk_keymap.py' for name resolution.
KEYCODES_JSON := $(BUILD_DIR)/keycodes.json

# [Custom Keycodes JSON]
# Mapping of user-defined enum keycodes (e.g., 0x7E40 -> SAFE_RANGE) from keymap.c.
# Type: src/types.py:KeycodesJson
# Generated from: 'generate_custom_keycodes.py' parsing 'keymap.c'.
# Used by: 'postprocess_qmk_keymap.py', 'generate_vitaly_layout.py' to preserve custom codes.
CUSTOM_KEYCODES_JSON := $(BUILD_DIR)/custom-keycodes.json

# [Vial JSON]
# VIAL-compatible keyboard definition (matrix, layout, VID/PID).
# Type: src/types.py:VialJson
# Generated from: 'generate_vial.py' using keyboard.json.
# Used by: 'qmk compile' (embedded in firmware) for VIAL support.
VIAL_JSON := $(BUILD_DIR)/vial.json

# [Vitaly JSON]
# Temporary dump of the keyboard's current VIAL configuration.
# Type: src/types.py:VitalyJson
# Generated from: 'vitaly save' (downloaded from device).
# Used by: 'generate_qmk_keymap_from_vitaly.py' (source for rebuild), 'generate_vitaly_layout.py' (base for merge).
VITALY_JSON := $(BUILD_DIR)/vitaly.json

# Same lazy-and-cached treatment as DEVICE_PID. These are only meaningful once
# $(QMK_KEYMAP_JSON) exists, which is why install/draw-layers build it in a
# first pass and then re-enter make to expand $(PNG).
LAYERS = $(eval LAYERS := $(shell if [ -s $(QMK_KEYMAP_JSON) ]; then $(UV) run python -m scripts.count_layers "$(QMK_KEYMAP_JSON)" || echo 0; else echo 0; fi))$(LAYERS)
PNG = $(eval PNG := $(shell if [ $(LAYERS) -gt 0 ]; then seq -f "$(BUILD_DIR)/$(KEYMAP_PREFIX)L%g.png" 0 $$(( $(LAYERS) - 1 )); fi))$(PNG)

endif

# ================= OVERLAY CONFIGURATION =================
KEYMAP_OVERLAY_DIR := $(HOME)/.config/keymap-overlay
KEYMAP_OVERLAY_LOG_DIR := $(HOME)/.local/var/log/keymap-overlay
KEYMAP_OVERLAY_BINARY := $(KEYMAP_OVERLAY_DIR)/keymap-overlay$(EXE_SUFFIX)
KEYMAP_OVERLAY_LABEL := com.sunaemon.keymap-overlay
KEYMAP_OVERLAY_PLIST := $(HOME)/Library/LaunchAgents/$(KEYMAP_OVERLAY_LABEL).plist
KEYMAP_OVERLAY_UNIT_NAME := keymap-overlay.service
KEYMAP_OVERLAY_UNIT := $(HOME)/.config/systemd/user/$(KEYMAP_OVERLAY_UNIT_NAME)
# The Task Scheduler counterpart. The task name doubles as the path of the
# folder it lives in, so it is a name and not a reverse-DNS label.
KEYMAP_OVERLAY_TASK_NAME := KeymapOverlay
KEYMAP_OVERLAY_TASK_XML := $(KEYMAP_OVERLAY_DIR)/keymap-overlay-task.xml
# One rule per keyboard, tagged uaccess so the logged-in user may open the Raw
# HID node; without it the overlay enumerates the keyboards but cannot read
# from them.
KEYMAP_OVERLAY_UDEV_RULES := /etc/udev/rules.d/50-keymap-overlay.rules
KEYMAP_OVERLAY ?= $(CARGO) run -p keymap-overlay --

# ================= TARGETS =================

.PHONY: all
all: draw-layers

.PHONY: format
format:
	$(MISE_DEV) run format

.PHONY: setup
setup:
	@$(MAKE) _setup_toolchain_$(OS_FAMILY)
	git submodule update --init --recursive
	$(MISE) trust
ifeq ($(OS_FAMILY),windows)
	# Windows users only need the runtime toolchain to build and run the
	# overlay. The formatting and firmware-development tools are unsupported
	# there, and several have no Windows distribution.
	$(MISE) install
else
# The dev tools come too: the git hooks installed below run format and lint.
	$(MISE_DEV) install
endif
	$(UV) sync
	@$(MAKE) install-hooks

.PHONY: _setup_toolchain_macos
_setup_toolchain_macos:
	$(BREW) tap osx-cross/arm
	$(BREW) tap osx-cross/avr
	@if $(BREW) list --versions arm-none-eabi-gcc >/dev/null; then \
		echo "ERROR: Homebrew core arm-none-eabi-gcc is incompatible with QMK."; \
		echo "Remove it with 'brew uninstall arm-none-eabi-gcc', then run make setup again."; \
		exit 1; \
	fi
	$(BREW) install $(QMK_TOOLCHAIN_PACKAGES)

# Distributions ship the compilers QMK wants, so there is no equivalent of the
# osx-cross taps here. libudev and the Wayland client library are the overlay's
# own build dependencies, not QMK's.
.PHONY: _setup_toolchain_linux
_setup_toolchain_linux:
	@if command -v pacman >/dev/null; then \
		set -x; $(SUDO) pacman -S --needed $(LINUX_TOOLCHAIN_PACKAGES_PACMAN); \
	elif command -v apt-get >/dev/null; then \
		set -x; $(SUDO) apt-get update && $(SUDO) apt-get install --yes $(LINUX_TOOLCHAIN_PACKAGES_APT); \
	elif command -v dnf >/dev/null; then \
		set -x; $(SUDO) dnf install --assumeyes $(LINUX_TOOLCHAIN_PACKAGES_DNF); \
	else \
		echo "ERROR: no supported package manager (pacman, apt-get, dnf) was found."; \
		echo "Install the ARM and AVR toolchains, libudev, and the Wayland client"; \
		echo "development files by hand, then run the rest of 'make setup'."; \
		exit 1; \
	fi

# There is no QMK toolchain to install here: firmware is built elsewhere (see
# the note this prints). What this does check is the two things every other
# Windows target assumes — cygpath, to hand native paths to native programs,
# and powershell, which registers the login task.
.PHONY: _setup_toolchain_windows
_setup_toolchain_windows:
	@missing=""; \
	for tool in cygpath powershell; do \
		command -v "$$tool" >/dev/null || missing="$$missing $$tool"; \
	done; \
	if [ -n "$$missing" ]; then \
		echo "ERROR: missing required command(s):$$missing"; \
		echo "Run 'make setup' from an MSYS2 or Git Bash shell on Windows, with"; \
		echo "the Windows PowerShell directory on PATH."; \
		exit 1; \
	fi
	@echo "NOTE: firmware is not built or flashed on Windows."
	@echo "      Use WSL, macOS or Linux for 'make compile', 'make flash' and"
	@echo "      'make flash-keymap'; see the Platform Support section of README.md."

.PHONY: doctor
doctor:
	@set -o pipefail; \
	$(QMK) doctor -n 2>&1 | sed '/QMK home does not appear to be a Git repository! (no .git folder)/d'; \
	status=$${PIPESTATUS[0]}; \
	[ "$$status" -eq 0 ] || [ "$$status" -eq 1 ] || exit "$$status"

# Because  LAYERS variable depends on $(QMK_KEYMAP_JSON), we need to call draw-layers with another make invocation
.PHONY: install
install:
ifdef KEYBOARD_ID
	@$(MAKE) $(QMK_KEYMAP_JSON)
	@$(MAKE) _internal_install
else
	@echo "KEYBOARD_ID not set, installing all keyboards..."
	@for kb in $(patsubst $(KEYBOARDS_DIR)/%/config.json,%,$(wildcard $(KEYBOARDS_DIR)/*/config.json)); do \
		echo "----------------------------------------------------------------"; \
		echo "Installing $$kb"; \
		$(MAKE) install KEYBOARD_ID=$$kb || exit 1; \
	done
endif

.PHONY: draw-layers
draw-layers:
ifdef KEYBOARD_ID
	@$(MAKE) $(QMK_KEYMAP_JSON)
	@$(MAKE) _internal_draw_layers
else
	@echo "KEYBOARD_ID not set, drawing layers for all keyboards..."
	@for kb in $(patsubst $(KEYBOARDS_DIR)/%/config.json,%,$(wildcard $(KEYBOARDS_DIR)/*/config.json)); do \
		echo "----------------------------------------------------------------"; \
		echo "Drawing layers for $$kb"; \
		$(MAKE) draw-layers KEYBOARD_ID=$$kb || exit 1; \
	done
endif

.PHONY: lint
lint:
	$(MISE_DEV) run lint

# lefthook writes into .git/hooks, which any core.hooksPath would shadow. The
# repo-local setting is ours to clear; one inherited from the user's global or
# system config is not, so stop and let them decide rather than unsetting it
# behind their back the way `lefthook install --reset-hooks-path` would.
.PHONY: install-hooks
install-hooks:
	@if [ -n "$$(git config --local --get core.hooksPath)" ]; then \
		git config --local --unset core.hooksPath; \
	fi
	@if [ -n "$$(git config --get core.hooksPath)" ]; then \
		echo "ERROR: core.hooksPath is set outside this repository:"; \
		echo "  $$(git config --show-origin --get core.hooksPath)"; \
		echo "It would shadow the hooks lefthook installs into .git/hooks."; \
		echo "Unset it, or accept losing it with:"; \
		echo "  $(LEFTHOOK) install --reset-hooks-path"; \
		exit 1; \
	fi
	$(LEFTHOOK) install

.PHONY: uninstall-hooks
uninstall-hooks:
	$(LEFTHOOK) uninstall

# Run by the lefthook commit-msg hook, which passes git's message file.
.PHONY: check-commit-message
check-commit-message:
ifndef COMMIT_MSG_FILE
	$(error COMMIT_MSG_FILE is required for check-commit-message)
endif
	@$(UV) run python -m scripts.check_commit_message "$(COMMIT_MSG_FILE)"

# Checks Cargo.lock against the RustSec advisory database. Ignored advisories
# and the reasons for them live in .cargo/audit.toml.
.PHONY: audit
audit:
	$(CARGO_AUDIT) audit

.PHONY: test
test:
	$(UV) run pytest

.PHONY: test-rust
test-rust:
	$(CARGO) test --workspace

.PHONY: clean
clean:
	rm -rf build

.PHONY: run-overlay
run-overlay:
	$(KEYMAP_OVERLAY) "$(KEYMAP_OVERLAY_DIR)"

.PHONY: build-overlay
build-overlay:
	$(CARGO) build --release -p keymap-overlay

.PHONY: install-overlay
install-overlay: install build-overlay
	@mkdir -p "$(KEYMAP_OVERLAY_DIR)" "$(KEYMAP_OVERLAY_LOG_DIR)"
# Windows holds an open executable locked, so the running overlay has to go
# before its binary can be replaced. The other two systems replace the file
# underneath the running process and stop it as part of installing the service.
	@$(MAKE) _stop_service_$(OS_FAMILY)
	install -C target/release/keymap-overlay$(EXE_SUFFIX) "$(KEYMAP_OVERLAY_BINARY)"
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
	@$(STOP_KEYMAP_OVERLAY_TASK)

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
		'  </array>' \
		'  <key>RunAtLoad</key>' \
		'  <true/>' \
		'  <key>KeepAlive</key>' \
		'  <dict><key>SuccessfulExit</key><false/></dict>' \
		'  <key>ProcessType</key>' \
		'  <string>Interactive</string>' \
		'  <key>EnvironmentVariables</key>' \
		'  <dict>' \
		'    <key>KEYMAP_OVERLAY_LOG_DIR</key>' \
		'    <string>$(call xml_escape,$(KEYMAP_OVERLAY_LOG_DIR))</string>' \
		'  </dict>' \
		'</dict>' \
		'</plist>'; \
		} > "$(KEYMAP_OVERLAY_PLIST).tmp" && mv "$(KEYMAP_OVERLAY_PLIST).tmp" "$(KEYMAP_OVERLAY_PLIST)"
	@$(BOOTOUT_KEYMAP_OVERLAY)
	launchctl bootstrap "gui/$$(id -u)" "$(KEYMAP_OVERLAY_PLIST)"

# The paths are written out rather than expressed with %h so that a
# KEYMAP_OVERLAY_DIR pointing elsewhere is honoured, and quoted so that a
# directory containing spaces still parses.
.PHONY: _install_service_linux
_install_service_linux:
	@mkdir -p "$(dir $(KEYMAP_OVERLAY_UNIT))"
	@{ \
		printf '%s\n' \
		'[Unit]' \
		'Description=QMK keymap layer overlay' \
		'Documentation=https://github.com/sunaemon/keymap-overlay' \
		'# The overlay is a Wayland layer surface, so it belongs to the' \
		'# graphical session and goes away with it.' \
		'PartOf=graphical-session.target' \
		'After=graphical-session.target' \
		'# The compositor may still be coming up when the session target is' \
		'# reached, and each retry is a start; without this the rate limiter' \
		'# would give up before it is listening.' \
		'StartLimitIntervalSec=0' \
		'' \
		'[Service]' \
		'Type=simple' \
		'ExecStart="$(KEYMAP_OVERLAY_BINARY)" "$(KEYMAP_OVERLAY_DIR)"' \
		'Environment="KEYMAP_OVERLAY_LOG_DIR=$(KEYMAP_OVERLAY_LOG_DIR)"' \
		'# Matches KeepAlive/SuccessfulExit=false in the launchd plist:' \
		'# come back after a crash, stay stopped after a clean exit.' \
		'Restart=on-failure' \
		'RestartSec=2' \
		'' \
		'[Install]' \
		'WantedBy=graphical-session.target'; \
		} > "$(KEYMAP_OVERLAY_UNIT).tmp" && mv "$(KEYMAP_OVERLAY_UNIT).tmp" "$(KEYMAP_OVERLAY_UNIT)"
	systemctl --user daemon-reload
	systemctl --user enable "$(KEYMAP_OVERLAY_UNIT_NAME)"
# restart, not start: this is also the update path, and the running process
# still holds the previous binary.
	systemctl --user restart "$(KEYMAP_OVERLAY_UNIT_NAME)"

# Task Scheduler is the one per-user "start this at login" mechanism Windows
# offers that also brings a crashed process back. Three of its defaults would
# otherwise stop the overlay and are set explicitly: tasks are killed after
# three days, stopped when the machine goes on battery, and not started at all
# while on battery.
#
# It has no equivalent of the plist's EnvironmentVariables or the unit's
# Environment, so KEYMAP_OVERLAY_LOG_DIR cannot travel to the task; the overlay
# falls back to the same directory under USERPROFILE, which is where this
# variable points anyway unless it was overridden.
.PHONY: _install_service_windows
_install_service_windows:
	@if [ "$(KEYMAP_OVERLAY_LOG_DIR)" != "$(HOME)/.local/var/log/keymap-overlay" ]; then \
		echo "ERROR: KEYMAP_OVERLAY_LOG_DIR cannot be honoured on Windows."; \
		echo "A scheduled task is given no environment, so the overlay would keep"; \
		echo "logging to its default directory. Leave the variable unset."; \
		exit 1; \
	fi
	@mkdir -p "$(dir $(KEYMAP_OVERLAY_TASK_XML))"
# cygpath because the task is run by Windows, which cannot follow an MSYS path,
# and the sed escapes a & or < that a Windows user name may contain.
	@xml_escape() { sed -e 's/&/\&amp;/g' -e 's/</\&lt;/g' -e 's/>/\&gt;/g'; }; \
	binary="$$(cygpath -w "$(KEYMAP_OVERLAY_BINARY)" | xml_escape)"; \
	assets="$$(cygpath -w "$(KEYMAP_OVERLAY_DIR)" | xml_escape)"; \
	{ \
	printf '%s\n' \
	'<?xml version="1.0"?>' \
	'<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">' \
	'  <RegistrationInfo>' \
	'    <Description>QMK keymap layer overlay</Description>' \
	'    <URI>\$(KEYMAP_OVERLAY_TASK_NAME)</URI>' \
	'  </RegistrationInfo>' \
	'  <Triggers><LogonTrigger><Enabled>true</Enabled></LogonTrigger></Triggers>' \
	'  <Principals>' \
	'    <Principal id="Author">' \
	'      <LogonType>InteractiveToken</LogonType>' \
	'      <RunLevel>LeastPrivilege</RunLevel>' \
	'    </Principal>' \
	'  </Principals>' \
	'  <Settings>' \
	'    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>' \
	'    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>' \
	'    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>' \
	'    <ExecutionTimeLimit>PT0S</ExecutionTimeLimit>' \
	'    <IdleSettings><StopOnIdleEnd>false</StopOnIdleEnd></IdleSettings>' \
	'    <AllowStartOnDemand>true</AllowStartOnDemand>' \
	'    <Enabled>true</Enabled>' \
	'    <RunOnlyIfIdle>false</RunOnlyIfIdle>' \
	'    <!-- Matches KeepAlive/SuccessfulExit=false in the launchd plist and' \
	'         Restart=on-failure in the systemd unit: a task is only retried' \
	'         when it exits non-zero, so a clean exit stays stopped. -->' \
	'    <RestartOnFailure><Interval>PT1M</Interval><Count>3</Count></RestartOnFailure>' \
	'  </Settings>' \
	'  <Actions Context="Author">' \
	'    <Exec>' \
	"      <Command>$$binary</Command>" \
	"      <Arguments>\"$$assets\"</Arguments>" \
	'    </Exec>' \
	'  </Actions>' \
	'</Task>'; \
	} > "$(KEYMAP_OVERLAY_TASK_XML).tmp" && mv "$(KEYMAP_OVERLAY_TASK_XML).tmp" "$(KEYMAP_OVERLAY_TASK_XML)"
# -Force is the update path: it replaces a task that is already registered.
	@xml="$$(cygpath -w "$(KEYMAP_OVERLAY_TASK_XML)")"; \
	MSYS2_ARG_CONV_EXCL='*' powershell.exe -NoProfile -NonInteractive -Command \
	"Register-ScheduledTask -TaskName '$(KEYMAP_OVERLAY_TASK_NAME)' -Xml (Get-Content -Raw -LiteralPath '$$xml') -Force | Out-Null"
	MSYS2_ARG_CONV_EXCL='*' powershell.exe -NoProfile -NonInteractive -Command \
		'Start-ScheduledTask -TaskName "$(KEYMAP_OVERLAY_TASK_NAME)"'

.PHONY: uninstall-overlay
uninstall-overlay:
	@$(MAKE) _uninstall_service_$(OS_FAMILY)
	rm -f "$(KEYMAP_OVERLAY_BINARY)"
	rm -f "$(KEYMAP_OVERLAY_DIR)"/*.png
	@echo "✔ Overlay service and installed assets removed; logs remain at $(KEYMAP_OVERLAY_LOG_DIR)"

.PHONY: _uninstall_service_macos
_uninstall_service_macos:
	@$(BOOTOUT_KEYMAP_OVERLAY)
	rm -f "$(KEYMAP_OVERLAY_PLIST)"

.PHONY: _uninstall_service_linux
_uninstall_service_linux:
	@$(STOP_KEYMAP_OVERLAY_UNIT)
	rm -f "$(KEYMAP_OVERLAY_UNIT)"
	systemctl --user daemon-reload

.PHONY: _uninstall_service_windows
_uninstall_service_windows:
	@$(STOP_KEYMAP_OVERLAY_TASK)
	@MSYS2_ARG_CONV_EXCL='*' powershell.exe -NoProfile -NonInteractive -Command \
		'Unregister-ScheduledTask -TaskName "$(KEYMAP_OVERLAY_TASK_NAME)" -Confirm:$$false -ErrorAction SilentlyContinue'
	rm -f "$(KEYMAP_OVERLAY_TASK_XML)"

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
		for kb in $(patsubst $(KEYBOARDS_DIR)/%/config.json,%,$(wildcard $(KEYBOARDS_DIR)/*/config.json)); do \
			$(MAKE) --no-print-directory print-udev-rule KEYBOARD_ID=$$kb || exit 1; \
		done; \
		} > build/keymap-overlay.rules.tmp && mv build/keymap-overlay.rules.tmp build/keymap-overlay.rules
	$(SUDO) install -m 0644 build/keymap-overlay.rules "$(KEYMAP_OVERLAY_UDEV_RULES)"
	$(SUDO) udevadm control --reload
	$(SUDO) udevadm trigger --subsystem-match=hidraw
	@echo "✔ Rules installed at $(KEYMAP_OVERLAY_UDEV_RULES); replug a keyboard if the overlay still cannot open it"

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
	@vid="$$( $(UV) run python -m scripts.get_keyboard_metadata "$(KEYBOARD_JSON)" vid)" || exit 1; \
		pid="$$( $(UV) run python -m scripts.get_keyboard_metadata "$(KEYBOARD_JSON)" pid)" || exit 1; \
		printf '\n# %s (KEYBOARD_ID=%s)\nKERNEL=="hidraw*", SUBSYSTEM=="hidraw", ATTRS{idVendor}=="%s", ATTRS{idProduct}=="%s", TAG+="uaccess"\n' \
		"$(QMK_KEYBOARD)" "$(KEYBOARD_ID)" "$$vid" "$$pid"

.PHONY: _copy_firmware
_copy_firmware:
ifndef KEYBOARD_ID
	$(error KEYBOARD_ID is required for _copy_firmware)
endif
	mkdir -p "$(QMK_HOME)/keyboards/$(QMK_KEYBOARD)/keymaps/$(QMK_KEYMAP)"
	mkdir -p "$(BUILD_DIR)"
	install -C $(KEYBOARDS_DIR)/$(KEYBOARD_ID)/config.h "$(QMK_HOME)/keyboards/$(QMK_KEYBOARD)/config.h"
	install -C $(KEYBOARDS_DIR)/$(KEYBOARD_ID)/keyboard.json "$(QMK_HOME)/keyboards/$(QMK_KEYBOARD)/keyboard.json"
	install -C firmware/layer_notify.h "$(QMK_HOME)/keyboards/$(QMK_KEYBOARD)/keymaps/$(QMK_KEYMAP)/layer_notify.h"
	$(call WRITE_OUTPUT,$(VIAL_JSON),$(UV) run python -m scripts.generate_vial --keyboard-json $(KEYBOARDS_DIR)/$(KEYBOARD_ID)/keyboard.json --layout-name "$(LAYOUT_NAME)")
	install -C $(VIAL_JSON) "$(QMK_HOME)/keyboards/$(QMK_KEYBOARD)/keymaps/$(QMK_KEYMAP)/vial.json"
	install -C $(KEYBOARDS_DIR)/$(KEYBOARD_ID)/keymap/* "$(QMK_HOME)/keyboards/$(QMK_KEYBOARD)/keymaps/$(QMK_KEYMAP)/"

.PHONY: compile
compile:
ifeq ($(OS_FAMILY),windows)
	$(error compile $(WINDOWS_FIRMWARE_ERROR))
endif
ifdef KEYBOARD_ID
	@$(MAKE) _copy_firmware
	$(QMK) compile -kb $(QMK_KEYBOARD) -km $(QMK_KEYMAP) $(QMK_FLAGS)
else
	@echo "KEYBOARD_ID not set, compiling all keyboards..."
	@for kb in $(patsubst $(KEYBOARDS_DIR)/%/config.json,%,$(wildcard $(KEYBOARDS_DIR)/*/config.json)); do \
		echo "----------------------------------------------------------------"; \
		echo "Compiling $$kb"; \
		$(MAKE) compile KEYBOARD_ID=$$kb || exit 1; \
	done
endif

.PHONY: flash
flash:
ifeq ($(OS_FAMILY),windows)
	$(error flash $(WINDOWS_FIRMWARE_ERROR))
endif
ifndef KEYBOARD_ID
	$(error KEYBOARD_ID is required for flash)
endif
	@$(MAKE) compile
	@$(MAKE) _flash_$(OS_FAMILY)

.PHONY: _flash_macos
_flash_macos:
	$(QMK) flash -kb $(QMK_KEYBOARD) -km $(QMK_KEYMAP) $(QMK_FLAGS)

# An rp2040 board flashes by copying a UF2 onto the mass storage volume its
# bootloader exposes, and qmk only ever looks for that volume already mounted
# (qmk_firmware/util/uf2conv.py). Nothing on a Linux box mounts it on its own:
# Plasma does not auto-mount by default, and udisks refuses a remote session
# or mounts under /run/media/root, so qmk waits forever. Mount it first, with
# sudo, and qmk's wait loop finds it immediately. Other bootloaders reach the
# board over USB with no filesystem in the way, so they skip this.
.PHONY: _flash_linux
_flash_linux:
	@bootloader="$$( $(UV) run python -m scripts.get_keyboard_metadata "$(KEYBOARD_JSON)" bootloader)" || exit 1; \
	case "$$bootloader" in \
		rp2040) $(MAKE) _mount_uf2_volume || exit 1 ;; \
	esac
	$(QMK) flash -kb $(QMK_KEYBOARD) -km $(QMK_KEYMAP) $(QMK_FLAGS)

.PHONY: _mount_uf2_volume
_mount_uf2_volume:
	@echo "Waiting for the $(UF2_VOLUME_LABEL) volume; put the board into its bootloader now..."
	$(UV) run python -m scripts.mount_uf2_volume --label "$(UF2_VOLUME_LABEL)" --sudo "$(SUDO)"

.PHONY: flash-keymap
flash-keymap:
ifeq ($(OS_FAMILY),windows)
	$(error flash-keymap $(WINDOWS_FIRMWARE_ERROR))
endif
ifeq ($(VIAL),true)
	$(error flash-keymap writes keymap.c to the device; VIAL=true would read the device and write it straight back)
endif
ifdef KEYBOARD_ID
	@$(MAKE) $(QMK_KEYMAP_JSON_RAW) $(CUSTOM_KEYCODES_JSON)
	@echo "Fetching current configuration from device..."
	$(VITALY) -i $(DEVICE_PID) save -f $(VITALY_JSON)
	@[ -s "$(VITALY_JSON)" ] || (echo "ERROR: No VIAL dump found at $(VITALY_JSON)"; exit 1)
	@echo "Merging QMK keymap into Vitaly configuration..."
	@# Uses the raw keymap, not $(QMK_KEYMAP_JSON): postprocessing resolves KC_TRNS
	@# for drawing, and writing those resolved keys to EEPROM would break inheritance.
	$(call WRITE_OUTPUT,$(BUILD_DIR)/vitaly_ready.json,$(UV) run python -m scripts.generate_vitaly_layout --qmk-keymap-json "$(QMK_KEYMAP_JSON_RAW)" --vitaly-json "$(VITALY_JSON)" --keyboard-json "$(KEYBOARDS_DIR)/$(KEYBOARD_ID)/keyboard.json" --custom-keycodes-json "$(CUSTOM_KEYCODES_JSON)" --layout-name "$(LAYOUT_NAME)")
	@echo "Loading new configuration to device..."
	$(VITALY) -i $(DEVICE_PID) load -f $(BUILD_DIR)/vitaly_ready.json
else
	@echo "KEYBOARD_ID not set, flashing all keyboards..."
	@for kb in $(patsubst $(KEYBOARDS_DIR)/%/config.json,%,$(wildcard $(KEYBOARDS_DIR)/*/config.json)); do \
		echo "----------------------------------------------------------------"; \
		echo "Flashing keymap for $$kb"; \
		$(MAKE) flash-keymap KEYBOARD_ID=$$kb || exit 1; \
	done
endif

.PHONY: patch-load
patch-load:
ifdef KEYBOARD_ID
	@echo "Loading keyboard configuration from $(QMK_HOME)..."
	cp "$(QMK_HOME)/keyboards/$(QMK_KEYBOARD)/config.h" "$(KEYBOARDS_DIR)/$(KEYBOARD_ID)/config.h"
	cp "$(QMK_HOME)/keyboards/$(QMK_KEYBOARD)/keyboard.json" "$(KEYBOARDS_DIR)/$(KEYBOARD_ID)/keyboard.json"
	cp -r "$(QMK_HOME)/keyboards/$(QMK_KEYBOARD)/keymaps/$(QMK_KEYMAP)/." "$(KEYBOARDS_DIR)/$(KEYBOARD_ID)/keymap/"
	@echo "✔ Keyboard configuration loaded"
else
	@echo "KEYBOARD_ID not set, patching all keyboards..."
	@for kb in $(patsubst $(KEYBOARDS_DIR)/%/config.json,%,$(wildcard $(KEYBOARDS_DIR)/*/config.json)); do \
		echo "----------------------------------------------------------------"; \
		echo "Patching $$kb"; \
		$(MAKE) patch-load KEYBOARD_ID=$$kb || exit 1; \
	done
endif

.PHONY: print-vars
print-vars:
	@echo "VIAL=$(VIAL)"
	@echo ""
	@echo "RSVG=$(RSVG)"
	@echo "MISE=$(MISE)"
	@echo "QMK=$(QMK)"
	@echo "KEYMAP=$(KEYMAP)"
	@echo "UV=$(UV)"
	@echo "VITALY=$(VITALY)"
	@echo ""
	@echo "QMK_HOME=$(QMK_HOME)"
	@echo "QMK_KEYBOARD=$(QMK_KEYBOARD)"
	@echo "KEYMAP_PREFIX=$(KEYMAP_PREFIX)"
	@echo "QMK_KEYMAP=$(QMK_KEYMAP)"
	@echo "KEYBOARD_JSON=$(KEYBOARD_JSON)"
	@echo "QMK_KEYMAP_C=$(QMK_KEYMAP_C)"
	@echo "DEVICE_PID=$(DEVICE_PID)"
	@echo "LAYOUT_NAME=$(LAYOUT_NAME)"
	@echo "DPI=$(DPI)"
	@echo ""
	@echo "BUILD_DIR=$(BUILD_DIR)"
	@echo "QMK_KEYMAP_JSON=$(QMK_KEYMAP_JSON)"
	@echo "KEYMAP_DRAWER_YAML=$(KEYMAP_DRAWER_YAML)"
	@echo "KEYCODES_JSON=$(KEYCODES_JSON)"
	@echo "CUSTOM_KEYCODES_JSON=$(CUSTOM_KEYCODES_JSON)"
	@echo "VIAL_JSON=$(VIAL_JSON)"
	@echo "VITALY_JSON=$(VITALY_JSON)"
	@echo "LAYERS=$(LAYERS)"
	@echo "PNG=$(PNG)"
	@echo ""
	@echo "KEYMAP_OVERLAY_DIR=$(KEYMAP_OVERLAY_DIR)"
	@echo "KEYBOARDS_DIR=$(KEYBOARDS_DIR)"

# ================= INTERNAL TARGETS =================

.PHONY: _internal_install
_internal_install: $(PNG)
	@if [ "$(LAYERS)" -eq "0" ]; then \
		echo "ERROR: No layers found even after generation."; \
		exit 1; \
	fi
	@echo "Installing keymap overlay assets..."
	@mkdir -p "$(KEYMAP_OVERLAY_DIR)"
	@cp $(BUILD_DIR)/$(KEYMAP_PREFIX)L*.png "$(KEYMAP_OVERLAY_DIR)/"
	@echo "✔ Overlay assets installed; run 'make run-overlay' to start the native app"

.PHONY: _internal_draw_layers
_internal_draw_layers: $(PNG)

# ================= FILE RULES =================

$(BUILD_DIR):
	mkdir -p $(BUILD_DIR)

$(BUILD_DIR)/%.png: $(BUILD_DIR)/%.svg
	$(RSVG) --dpi $(DPI) "$<" "$@"

$(BUILD_DIR)/$(KEYMAP_PREFIX)L%.svg: $(KEYMAP_DRAWER_YAML) | $(BUILD_DIR)
	$(call WRITE_OUTPUT,$@,$(KEYMAP) draw "$(KEYMAP_DRAWER_YAML)" -j "$(KEYBOARD_JSON)" -l "$(LAYOUT_NAME)" -s "L$*")

$(KEYMAP_DRAWER_YAML): $(QMK_KEYMAP_JSON) | $(BUILD_DIR)
	$(call WRITE_OUTPUT,$@,$(KEYMAP) parse -q $(QMK_KEYMAP_JSON))

.PHONY: _force_build
_force_build:

QMK_KEYMAP_JSON_RAW_DEPS := $(QMK_KEYMAP_C)
ifeq ($(VIAL),true)
QMK_KEYMAP_JSON_RAW_DEPS += _force_build
endif

QMK_KEYMAP_JSON_DEPS := scripts/postprocess_qmk_keymap.py $(CUSTOM_KEYCODES_JSON) $(QMK_KEYMAP_JSON_RAW)

$(QMK_KEYMAP_JSON_RAW): $(QMK_KEYMAP_JSON_RAW_DEPS) | $(BUILD_DIR)
ifeq ($(VIAL),true)
	@echo "Dumping QMK JSON from VIAL EEPROM..."
	$(VITALY) -i $(DEVICE_PID) save -f $(VITALY_JSON)
	@[ -s "$(VITALY_JSON)" ] || (echo "ERROR: No VIAL dump found at $(VITALY_JSON)"; exit 1)
	$(call WRITE_OUTPUT,$@,$(UV) run python -m scripts.generate_qmk_keymap_from_vitaly --vitaly-json $(VITALY_JSON) --keyboard-json "$(KEYBOARD_JSON)" --layout-name "$(LAYOUT_NAME)")
else
	@echo "Compiling QMK JSON from source..."
	$(call WRITE_OUTPUT,$@,$(QMK) c2json --no-cpp -kb $(QMK_KEYBOARD) -km $(QMK_KEYMAP) "$(QMK_KEYMAP_C)")
endif

$(QMK_KEYMAP_JSON): $(QMK_KEYMAP_JSON_DEPS) | $(BUILD_DIR)
	$(call WRITE_OUTPUT,$@,$(UV) run python -m scripts.postprocess_qmk_keymap "$(QMK_KEYMAP_JSON_RAW)" --custom-keycodes-json $(CUSTOM_KEYCODES_JSON))

$(KEYCODES_JSON): scripts/generate_keycodes.py | $(BUILD_DIR)
	$(call WRITE_OUTPUT,$@,$(UV) run python -m scripts.generate_keycodes --qmk-dir "$(QMK_HOME)")

$(CUSTOM_KEYCODES_JSON): $(QMK_KEYMAP_C) scripts/generate_custom_keycodes.py $(KEYCODES_JSON) | $(BUILD_DIR)
	$(call WRITE_OUTPUT,$@,$(UV) run python -m scripts.generate_custom_keycodes "$(QMK_KEYMAP_C)" --keycodes-json "$(KEYCODES_JSON)")

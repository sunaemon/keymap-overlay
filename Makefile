SHELL := /bin/bash

# ================= PLATFORM CONFIGURATION =================

# The image generation workflow is portable; the overlay's window, its login
# service, the toolchain packages, and the firmware workflow are not. Every
# target that differs between the systems dispatches on this.
#
# Windows development uses MSYS2 UCRT64 to drive a native Windows build.
# `uname -s` there reports MINGW64_NT-10.0-…, which no `ifeq` can match exactly,
# hence findstring. The MSYS match also keeps the recipes usable in CI.
# Compiling and flashing firmware is not supported on Windows — see
# `_setup_toolchain_windows`.
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

# Development model generation targets the current host.
OVERLAY_PLATFORM ?= $(OS_FAMILY)
ifeq (,$(filter $(OVERLAY_PLATFORM),macos linux windows))
$(error OVERLAY_PLATFORM must be macos, linux or windows, got '$(OVERLAY_PLATFORM)')
endif

# Cargo names the binary after the target, and the login service needs the name
# that exists on disk.
ifeq ($(OS_FAMILY),windows)
EXE_SUFFIX := .exe
else
EXE_SUFFIX :=
endif

# QMK's Windows toolchain is QMK MSYS, a separate environment from the one that
# builds the overlay. Keep QMK source processing and firmware deployment there;
# native Raw HID reads and writes use the portable Rust tooling below.
WINDOWS_FIRMWARE_ERROR := is not supported on Windows; compile and flash from WSL, macOS or Linux (see Platform Support in README.md)

# ================= VIA CONFIGURATION =================

# `make draw-layers` reads the connected keyboard's Vial EEPROM, reflecting
# live edits made in the Vial GUI rather than only compiled keymap.c source.
EEPROM_RESET_EPOCH ?= 0

# ================= TOOLS CONFIGURATION =================
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
UV ?= $(MISE) exec -- uv
CARGO ?= $(MISE) exec -- cargo

QMK_TOOLCHAIN_PACKAGES := osx-cross/arm/arm-none-eabi-gcc@8 osx-cross/avr/avr-gcc@9 avrdude dfu-programmer dfu-util

# The same set per distribution, plus libudev for Raw HID and the Qt 6 /
# LayerShellQt stack used by the native KDE Plasma overlay.
LINUX_TOOLCHAIN_PACKAGES_PACMAN := arm-none-eabi-gcc arm-none-eabi-binutils arm-none-eabi-newlib avr-gcc avr-libc avrdude dfu-programmer dfu-util systemd-libs cmake qt6-base qt6-declarative layer-shell-qt ttf-liberation
LINUX_TOOLCHAIN_PACKAGES_APT := gcc-arm-none-eabi binutils-arm-none-eabi libnewlib-arm-none-eabi gcc-avr avr-libc avrdude dfu-programmer dfu-util libudev-dev cmake qt6-base-dev qt6-declarative-dev qt6-wayland qml6-module-qtqml-workerscript qml6-module-qtquick qml6-module-qtquick-window fonts-liberation
LINUX_LAYERSHELL_QML_APT := qml6-module-org-kde-layershell
LINUX_TOOLCHAIN_PACKAGES_DNF := arm-none-eabi-gcc-cs arm-none-eabi-newlib avr-gcc avr-libc avrdude dfu-programmer dfu-util systemd-devel cmake qt6-qtbase-devel qt6-qtdeclarative-devel qt6-qtwayland layer-shell-qt liberation-mono-fonts

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

define STOP_KEYMAP_OVERLAY_QT_UNIT
if [ -f "$(KEYMAP_OVERLAY_QT_UNIT)" ]; then \
	systemctl --user disable --now "$(KEYMAP_OVERLAY_QT_UNIT_NAME)"; \
	fi
endef

# The Windows Run key is per-user, so it needs no administrator access. Stop
# the previous process before replacing its executable, if it is running. The
# command is single-quoted so that the shell leaves PowerShell's $ alone, and
# MSYS2_ARG_CONV_EXCL stops MSYS2 rewriting the arguments as paths.
#
# Run through `env` so the line does not open with NAME=VALUE, which the
# Makefile formatter rewrites to NAME = VALUE — turning the variable this needs
# in the environment into a command it would try to run.
#
# Stop-Process only asks the process to terminate; Windows holds the image lock
# until it is gone, so the caller has to wait or the copy that follows hits a
# sharing violation. Wait-Process supplies the wait, and the whole pipeline is
# a no-op when nothing is running.
define STOP_KEYMAP_OVERLAY_PROCESS
env MSYS2_ARG_CONV_EXCL='*' powershell.exe -NoProfile -NonInteractive -Command \
	'Get-Process -Name "keymap-overlay" -ErrorAction SilentlyContinue | Stop-Process -PassThru | Wait-Process -Timeout 10; exit 0'
endef

# ================= QMK CONFIGURATION =================
QMK_HOME := firmware/vendor/vial-qmk
export QMK_HOME := $(QMK_HOME)

QMK_KEYMAP ?= keymap
QMK_FLAGS += -e SKIP_GIT=yes

KEYBOARDS_DIR ?= firmware/examples
# The installed service passes this to the overlay for startup refresh
# (FILE RULES); it needs to survive being launched with an unrelated cwd.
KEYBOARDS_DIR_ABS := $(abspath $(KEYBOARDS_DIR))

# Every configured keyboard, by KEYBOARD_ID. A directory counts once it has a
# config.json, which is the file the ID and QMK_KEYBOARD are read from.
ALL_KEYBOARD_IDS = $(patsubst $(KEYBOARDS_DIR)/%/config.json,%,$(wildcard $(KEYBOARDS_DIR)/*/config.json))

# Re-enter make once per keyboard, for the targets that work on one at a time
# and were given no KEYBOARD_ID. $(1) is the verb for the banner, $(2) the verb
# for each keyboard, $(3) the target; the two verbs are separate because
# "flashing all keyboards" goes with "Flashing keymap for 1".
define FOR_EACH_KEYBOARD
echo "KEYBOARD_ID not set, $(1) all keyboards..."; \
for kb in $(ALL_KEYBOARD_IDS); do \
echo "----------------------------------------------------------------"; \
echo "$(2) $$kb"; \
$(MAKE) $(3) KEYBOARD_ID=$$kb || exit 1; \
done
endef

ifdef KEYBOARD_ID

# KEYBOARD_ID names a directory in $(KEYBOARDS_DIR), is compiled into the
# firmware as -DKEYBOARD_ID, and travels in one byte of the Raw HID report,
# so it has to be an integer that fits in a uint8_t.
ifneq ($(shell printf '%s' "$(KEYBOARD_ID)" | grep -Eq '^([0-9]|[1-9][0-9]|1[0-9][0-9]|2[0-4][0-9]|25[0-5])$$' && echo ok),ok)
    $(error KEYBOARD_ID must be an integer between 0 and 255, got '$(KEYBOARD_ID)')
endif

# QMK keyboard name (e.g., salicylic_acid3/insixty_en).
QMK_KEYBOARD ?= $(shell awk -F'"' '/qmk_keyboard/ {print $$4}' $(KEYBOARDS_DIR)/$(KEYBOARD_ID)/config.json)
ifeq ($(QMK_KEYBOARD),)
    $(error KEYBOARD_ID=$(KEYBOARD_ID) is not valid or $(KEYBOARDS_DIR)/$(KEYBOARD_ID)/config.json is missing or malformed)
endif
QMK_FLAGS += -e KEYBOARD_ID=$(KEYBOARD_ID)
QMK_FLAGS += -e KEYMAP_EEPROM_EPOCH=$(EEPROM_RESET_EPOCH)

# QMK keyboard definition (matrix/layouts/metadata).
# Type: model/src/types.py:KeyboardJson
KEYBOARD_JSON := $(KEYBOARDS_DIR)/$(KEYBOARD_ID)/keyboard.json
QMK_KEYMAP_C := $(KEYBOARDS_DIR)/$(KEYBOARD_ID)/keymap/keymap.c
KEYBOARD_CONFIG := $(KEYBOARDS_DIR)/$(KEYBOARD_ID)/config.json

# Evaluated on first use and then cached, so that targets which never need it
# (clean, compile, flash, print-vars) do not pay for a Python startup.
DEVICE_PID = $(eval DEVICE_PID := $(shell $(UV) run python -c "import json; print(int(json.load(open('$(KEYBOARD_JSON)'))['usb']['pid'], 16))"))$(DEVICE_PID)

LAYOUT_NAME := LAYOUT

PIXELS_PER_UNIT ?= 64

# ================= BUILD CONFIGURATION =================
BUILD_DIR := build/$(KEYBOARD_ID)
ABS_BUILD_DIR := $(abspath $(BUILD_DIR))

QMK_FLAGS += -e BUILD_DIR=$(ABS_BUILD_DIR)/qmk_build

# VIAL-compatible keyboard definition (matrix, layout, VID/PID, customKeycodes).
# Type: model/src/types.py:VialJson
# Generated from: 'generate_vial.py' using keyboard.json and keymap.c.
# Used by: 'qmk compile' (embedded in firmware) for VIAL support.
VIAL_JSON := $(BUILD_DIR)/vial.json

ASSET_EXTENSION := json
ASSET_BUILD_DIR := $(BUILD_DIR)/assets/$(OVERLAY_PLATFORM)
CONSOLIDATED_ASSET := $(ASSET_BUILD_DIR)/$(KEYBOARD_ID).$(ASSET_EXTENSION)

endif

# ================= OVERLAY CONFIGURATION =================
ifeq ($(OS_FAMILY),windows)
# MSYS2's HOME is private to MSYS2, so installed paths come from the Windows
# environment instead. Expanded on every invocation, not just `make setup`, so
# they cannot lean on _setup_toolchain_windows having checked for cygpath: left
# empty they would root every path at /, and `make uninstall-overlay` would
# delete from /keymap-overlay.
WINDOWS_USER_HOME := $(shell cygpath -u "$$USERPROFILE" 2>/dev/null)
ifeq ($(strip $(WINDOWS_USER_HOME)),)
$(error Could not resolve USERPROFILE with cygpath; run make from an MSYS2 UCRT64 shell)
endif
# Local rather than roaming %APPDATA%, because the log describes one machine.
WINDOWS_LOCAL_APP_DATA := $(shell cygpath -u "$$LOCALAPPDATA" 2>/dev/null)
ifeq ($(strip $(WINDOWS_LOCAL_APP_DATA)),)
WINDOWS_LOCAL_APP_DATA := $(WINDOWS_USER_HOME)/AppData/Local
endif
KEYMAP_OVERLAY_LOG_DIR ?= $(WINDOWS_LOCAL_APP_DATA)/keymap-overlay/logs
# Where a per-user install puts an executable on Windows, the same place VS Code
# and Slack use. The Run key names it by absolute path, so it need not be on
# PATH.
KEYMAP_OVERLAY_BIN_DIR ?= $(WINDOWS_LOCAL_APP_DATA)/Programs/keymap-overlay
else
KEYMAP_OVERLAY_LOG_DIR := $(HOME)/.local/var/log/keymap-overlay
# systemd puts this on PATH for user services and the distro profiles add it for
# login shells, so `keymap-overlay-qt` can be run by hand to diagnose the
# renderer instead of only by absolute path out of its unit.
KEYMAP_OVERLAY_BIN_DIR := $(HOME)/.local/bin
endif
KEYMAP_OVERLAY_LOG_FILE := $(KEYMAP_OVERLAY_LOG_DIR)/overlay.log
KEYMAP_OVERLAY_BINARY := $(KEYMAP_OVERLAY_BIN_DIR)/keymap-overlay$(EXE_SUFFIX)
KEYMAP_OVERLAY_QT_BINARY := $(KEYMAP_OVERLAY_BIN_DIR)/keymap-overlay-qt
WPF_PROJECT := overlay/platforms/windows/wpf/KeymapOverlay.Wpf.csproj
WPF_PUBLISH_DIR := target/wpf-publish
WINDOWS_BRIDGE_MANIFEST := overlay/platforms/windows/bridge/Cargo.toml
WINUI_PACKAGE := keymap-overlay-winui
QT_RENDERER_SOURCE := overlay/platforms/linux/qt
QT_RENDERER_BUILD_DIR := target/qt-release
ifeq ($(OS_FAMILY),windows)
WINDOWS_PROCESSOR_ARCHITECTURE := $(shell powershell -NoProfile -Command '(Get-ItemPropertyValue -LiteralPath "Registry::HKEY_LOCAL_MACHINE\SYSTEM\CurrentControlSet\Control\Session Manager\Environment" -Name PROCESSOR_ARCHITECTURE)')
ifeq ($(WINDOWS_PROCESSOR_ARCHITECTURE),ARM64)
WINDOWS_DOTNET_RID := win-arm64
else ifeq ($(WINDOWS_PROCESSOR_ARCHITECTURE),AMD64)
WINDOWS_DOTNET_RID := win-x64
else
$(error native Windows builds require AMD64 or ARM64, got '$(WINDOWS_PROCESSOR_ARCHITECTURE)')
endif
OVERLAY_BUILD_BINARY := $(WPF_PUBLISH_DIR)/keymap-overlay.exe
KEYMAP_OVERLAY ?= "$(OVERLAY_BUILD_BINARY)"
else
ifeq ($(OS_FAMILY),macos)
OVERLAY_PACKAGE := keymap-overlay-macos
else
OVERLAY_PACKAGE := keymap-overlay-linux-daemon
endif
OVERLAY_BUILD_BINARY := target/release/keymap-overlay
KEYMAP_OVERLAY ?= $(CARGO) run -p $(OVERLAY_PACKAGE) --
endif
KEYMAP_OVERLAY_LABEL := com.sunaemon.keymap-overlay
KEYMAP_OVERLAY_PLIST := $(HOME)/Library/LaunchAgents/$(KEYMAP_OVERLAY_LABEL).plist
KEYMAP_OVERLAY_UNIT_NAME := keymap-overlay.service
KEYMAP_OVERLAY_UNIT := $(HOME)/.config/systemd/user/$(KEYMAP_OVERLAY_UNIT_NAME)
KEYMAP_OVERLAY_QT_UNIT_NAME := keymap-overlay-qt.service
KEYMAP_OVERLAY_QT_UNIT := $(HOME)/.config/systemd/user/$(KEYMAP_OVERLAY_QT_UNIT_NAME)
GNOME_EXTENSION_UUID := keymap-overlay@sunaemon
GNOME_EXTENSION_SOURCE := overlay/platforms/linux/gnome-shell/$(GNOME_EXTENSION_UUID)
GNOME_EXTENSION_DIR := $(HOME)/.local/share/gnome-shell/extensions/$(GNOME_EXTENSION_UUID)
# The registry value under the current user's Run key that starts the overlay
# when they sign in. It is intentionally a user-level autostart, not a service.
KEYMAP_OVERLAY_RUN_VALUE := KeymapOverlay
# One rule per keyboard, tagged uaccess so the logged-in user may open the Raw
# HID node; without it the overlay enumerates the keyboards but cannot read
# from them.
KEYMAP_OVERLAY_UDEV_RULES := /etc/udev/rules.d/50-keymap-overlay.rules
DOTNET ?= $(MISE) exec -- dotnet
CMAKE ?= cmake

# ================= TARGETS =================

.PHONY: all
all: draw-layers

.PHONY: format
format:
	$(MISE_DEV) run format

.PHONY: setup
setup:
	@$(MAKE) _setup_toolchain_$(OS_FAMILY)
ifneq ($(OS_FAMILY),windows)
	@$(MAKE) setup-firmware
endif
	$(MISE) trust
ifeq ($(OS_FAMILY),windows)
	# Assets are generated in WSL. Installing just Rust and lefthook keeps the
	# native Windows setup independent of QMK and Python tooling.
	$(MISE) install rust dotnet lefthook
else
# The dev tools come too: the git hooks installed below run format and lint.
	$(MISE_DEV) install
	$(UV) sync
endif
	@$(MAKE) install-hooks

# Resolve nested dependencies from the configured processors. Unknown
# processors deliberately fall back to all submodules: a larger checkout is
# safer than guessing and leaving a newly added keyboard unable to compile.
.PHONY: setup-firmware
setup-firmware:
ifeq ($(OS_FAMILY),windows)
	$(error setup-firmware $(WINDOWS_FIRMWARE_ERROR))
endif
	git submodule update --init --checkout --depth 1 "$(QMK_HOME)"
	@submodules="$$( $(UV) run python -m firmware.tools.resolve_qmk_submodules "$(KEYBOARDS_DIR)" )" || exit 1; \
	case "$$submodules" in \
		--recursive) git -C "$(QMK_HOME)" submodule update --init --depth 1 --recursive ;; \
		*) git -C "$(QMK_HOME)" submodule update --init --depth 1 $$submodules ;; \
	esac

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
# osx-cross taps here. libudev, Qt Quick, Qt D-Bus, and LayerShellQt are the
# overlay's own build and runtime dependencies, not QMK's.
.PHONY: _setup_toolchain_linux
_setup_toolchain_linux:
	@if command -v pacman >/dev/null; then \
		set -x; $(SUDO) pacman -S --needed $(LINUX_TOOLCHAIN_PACKAGES_PACMAN); \
	elif command -v apt-get >/dev/null; then \
		set -e; set -x; $(SUDO) apt-get update && $(SUDO) apt-get install --yes $(LINUX_TOOLCHAIN_PACKAGES_APT); \
		if apt-cache show $(LINUX_LAYERSHELL_QML_APT) >/dev/null 2>&1; then \
			$(SUDO) apt-get install --yes $(LINUX_LAYERSHELL_QML_APT); \
		else \
			echo "WARNING: $(LINUX_LAYERSHELL_QML_APT) is unavailable; install the Qt 6 LayerShellQt QML module manually."; \
		fi; \
	elif command -v dnf >/dev/null; then \
		set -x; $(SUDO) dnf install --assumeyes $(LINUX_TOOLCHAIN_PACKAGES_DNF); \
	else \
		echo "ERROR: no supported package manager (pacman, apt-get, dnf) was found."; \
		echo "Install the ARM and AVR toolchains, libudev, Qt 6 Quick, and"; \
		echo "LayerShellQt by hand, then run the rest of 'make setup'."; \
		exit 1; \
	fi

# There is no QMK toolchain to install here: firmware is built elsewhere (see
# the note this prints). What this does check is the two things every other
# Windows target assumes — cygpath, to hand native paths to native programs,
# and powershell, which writes the current user's login Run key.
.PHONY: _setup_toolchain_windows
_setup_toolchain_windows:
	@missing=""; \
	for tool in cygpath powershell; do \
		command -v "$$tool" >/dev/null || missing="$$missing $$tool"; \
	done; \
	if [ -n "$$missing" ]; then \
		echo "ERROR: missing required command(s):$$missing"; \
		echo "Run 'make setup' from an MSYS2 UCRT64 shell on Windows, with"; \
		echo "the Windows PowerShell directory on PATH."; \
		exit 1; \
	fi
	@echo "NOTE: firmware and QMK source processing do not run in this shell."
	@echo "      Use WSL, macOS or Linux for 'make compile', 'make flash', and"
	@echo "      other QMK-backed targets; native Raw HID targets run here."

.PHONY: doctor
doctor:
	@set -o pipefail; \
	$(QMK) doctor -n 2>&1 | sed '/QMK home does not appear to be a Git repository! (no .git folder)/d'; \
	status=$${PIPESTATUS[0]}; \
	[ "$$status" -eq 0 ] || [ "$$status" -eq 1 ] || exit "$$status"

.PHONY: draw-layers
draw-layers:
ifdef KEYBOARD_ID
	@$(MAKE) _internal_draw_layers
else
	+@$(call FOR_EACH_KEYBOARD,drawing layers for,Drawing layers for,draw-layers)
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
	@$(UV) run python -m tools.check_commit_message "$(COMMIT_MSG_FILE)"

# Checks Cargo.lock against the RustSec advisory database. Ignored advisories
# and the reasons for them live in .cargo/audit.toml.
.PHONY: audit
audit:
	$(CARGO_AUDIT) audit

# Updates both manifests and every generated file that embeds their version.
.PHONY: bump-version
bump-version:
ifndef VERSION
	$(error VERSION is required, for example: make bump-version VERSION=0.0.5)
endif
	$(UV) run python -m installer.release.bump_version "$(VERSION)"

# This file ships beside every release binary. The non-mutating CI check keeps
# additions and upgrades in Cargo.lock from silently dropping their notices.
.PHONY: licenses
licenses:
	$(MISE_DEV) exec -- uv run python -m installer.release.generate_license_report

.PHONY: check-licenses
check-licenses:
	$(MISE_DEV) exec -- uv run python -m installer.release.generate_license_report --check

.PHONY: test
test:
	$(UV) run pytest

.PHONY: test-installer-sh
test-installer-sh:
	./installer/tests/test_install_sh.sh

.PHONY: test-installer-ps
test-installer-ps:
	powershell -NoProfile -Command 'Invoke-Pester -Path installer/tests/install.Tests.ps1 -CI'

.PHONY: test-rust
test-rust:
	$(CARGO) test --workspace

# Linux's release go/no-go gate. The installer test covers upgrade rollback;
# the two E2E halves meet at the typed D-Bus contract the project owns.
.PHONY: test-release-acceptance-linux
test-release-acceptance-linux: test-installer-sh test-hid-to-dbus-e2e-linux test-dbus-to-renderer-e2e-linux

# macOS's release go/no-go gate. HID behavior is exercised by the shared
# runtime and Linux UHID test; this covers the native AppKit presentation path.
.PHONY: test-release-acceptance-macos
test-release-acceptance-macos: test-installer-sh test-appkit-e2e-macos

# Windows's release go/no-go gate. The installer test covers upgrade rollback;
# the simulated E2E test covers the native WPF presentation path.
.PHONY: test-release-acceptance-windows
test-release-acceptance-windows: test-installer-ps test-wpf-e2e-windows

.PHONY: test-wpf-e2e-windows
test-wpf-e2e-windows: build-overlay
ifeq ($(OS_FAMILY),windows)
	powershell -NoProfile -ExecutionPolicy Bypass -File overlay/platforms/windows/tests/test_wpf_e2e.ps1
else
	$(error test-wpf-e2e-windows is only available on Windows)
endif

.PHONY: test-appkit-e2e-macos
test-appkit-e2e-macos: build-overlay
ifeq ($(OS_FAMILY),macos)
	./overlay/platforms/macos/tests/test_appkit_e2e.sh
else
	$(error test-appkit-e2e-macos is only available on macOS)
endif

.PHONY: test-dbus-to-renderer-e2e-linux
test-dbus-to-renderer-e2e-linux: build-overlay
ifeq ($(OS_FAMILY),linux)
	dbus-run-session -- ./overlay/platforms/linux/tests/test_dbus_to_renderer_e2e.sh
else
	$(error test-dbus-to-renderer-e2e-linux is only available on Linux)
endif

.PHONY: test-hid-to-dbus-e2e-linux
test-hid-to-dbus-e2e-linux: build-overlay
ifeq ($(OS_FAMILY),linux)
	$(CC) -o target/virtual-raw-hid \
		-std=c11 -Wall -Wextra -Wpedantic -Werror \
		overlay/platforms/linux/tests/virtual_raw_hid.c
	dbus-run-session -- ./overlay/platforms/linux/tests/test_hid_to_dbus_e2e.sh
else
	$(error test-hid-to-dbus-e2e-linux is only available on Linux)
endif

.PHONY: clean
clean:
	rm -rf build

.PHONY: run-overlay
run-overlay:
	$(KEYMAP_OVERLAY) $(if $(SIMULATE),--simulate "$(SIMULATE)")

.PHONY: build-overlay
build-overlay:
ifeq ($(OS_FAMILY),windows)
	$(CARGO) build --release --manifest-path "$(WINDOWS_BRIDGE_MANIFEST)" --target-dir target
	$(DOTNET) publish "$(WPF_PROJECT)" --configuration Release \
		--runtime "$(WINDOWS_DOTNET_RID)" --output "$(WPF_PUBLISH_DIR)"
else
	$(CARGO) build --release -p $(OVERLAY_PACKAGE)
ifeq ($(OS_FAMILY),linux)
	$(CMAKE) -S "$(QT_RENDERER_SOURCE)" -B "$(QT_RENDERER_BUILD_DIR)" \
		-DCMAKE_BUILD_TYPE=Release \
		-DCMAKE_RUNTIME_OUTPUT_DIRECTORY="$(abspath target/release)"
	$(CMAKE) --build "$(QT_RENDERER_BUILD_DIR)" --config Release
endif
endif

# Experimental pure-Rust WinUI 3 frontend. The WPF build above remains the
# installed and released Windows implementation until focus and transparency
# behavior have been verified on a real desktop.
.PHONY: build-winui-overlay
build-winui-overlay:
ifeq ($(OS_FAMILY),windows)
	$(CARGO) build --release -p "$(WINUI_PACKAGE)"
else
	$(error build-winui-overlay is only available on Windows)
endif

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
	launchctl bootstrap "gui/$$(id -u)" "$(KEYMAP_OVERLAY_PLIST)"

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
# The Windows frontend is WPF, which reaches the shared runtime through a C ABI
# that deliberately carries no strings, so it cannot be handed a `--log-out`
# path the way the plist hands one to the native binary. It writes to the
# default file under %LOCALAPPDATA% instead, which is where this variable
# points unless it was overridden.
.PHONY: _install_service_windows
_install_service_windows:
	@if [ "$(KEYMAP_OVERLAY_LOG_DIR)" != "$(WINDOWS_LOCAL_APP_DATA)/keymap-overlay/logs" ]; then \
		echo "ERROR: KEYMAP_OVERLAY_LOG_DIR cannot be honoured on Windows."; \
		echo "The WPF frontend takes no log argument, so the overlay would keep"; \
		echo "logging to its default directory. Leave the variable unset."; \
		exit 1; \
	fi
# set -e so a failing cygpath does not hand an empty path to the registry, and
# $ErrorActionPreference so PowerShell's non-terminating errors become failures
# make can see: without it, Set-ItemProperty or Start-Process can fail while
# powershell.exe still exits 0 and install-overlay reports success.
	@set -e; \
	binary="$$(cygpath -w "$(KEYMAP_OVERLAY_BINARY)")"; \
	env KEYMAP_OVERLAY_BINARY="$$binary" MSYS2_ARG_CONV_EXCL='*' powershell.exe -NoProfile -NonInteractive -Command \
	'$$ErrorActionPreference = "Stop"; $$quote = [char]34; $$command = $$quote + $$env:KEYMAP_OVERLAY_BINARY + $$quote; Set-ItemProperty -Path "HKCU:\Software\Microsoft\Windows\CurrentVersion\Run" -Name "$(KEYMAP_OVERLAY_RUN_VALUE)" -Value $$command; Start-Process -FilePath $$env:KEYMAP_OVERLAY_BINARY'

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
	$(call WRITE_OUTPUT,$(VIAL_JSON),$(UV) run python -m model.scripts.generate_vial --keyboard-json $(KEYBOARDS_DIR)/$(KEYBOARD_ID)/keyboard.json --keyboard-config $(KEYBOARDS_DIR)/$(KEYBOARD_ID)/config.json --keyboard-id $(KEYBOARD_ID) --pixels-per-unit $(PIXELS_PER_UNIT) --layout-name "$(LAYOUT_NAME)" --keymap-c "$(QMK_KEYMAP_C)")
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
	+@$(call FOR_EACH_KEYBOARD,compiling,Compiling,compile)
endif

.PHONY: flash
flash:
ifeq ($(OS_FAMILY),windows)
	$(error flash $(WINDOWS_FIRMWARE_ERROR))
endif
ifndef KEYBOARD_ID
	$(error KEYBOARD_ID is required for flash)
endif
	@epoch="$$(date +%s)"; \
	$(MAKE) compile EEPROM_RESET_EPOCH="$$epoch" && \
	$(MAKE) _flash_$(OS_FAMILY) EEPROM_RESET_EPOCH="$$epoch"

.PHONY: _flash_macos
_flash_macos:
	$(QMK) flash -kb $(QMK_KEYBOARD) -km $(QMK_KEYMAP) $(QMK_FLAGS)

# An rp2040 board flashes by copying a UF2 onto the mass storage volume its
# bootloader exposes, and qmk only ever looks for that volume already mounted
# (firmware/vendor/vial-qmk/util/uf2conv.py). Nothing on a Linux box mounts it on its own:
# Plasma does not auto-mount by default, and udisks refuses a remote session
# or mounts under /run/media/root, so qmk waits forever. Mount it first, with
# sudo, and qmk's wait loop finds it immediately. WSL keeps the USB mass-storage
# device attached after the copy, so unmount it once qmk finishes. Other
# bootloaders reach the board over USB with no filesystem in the way, so they
# skip both steps.
.PHONY: _flash_linux
_flash_linux:
	@bootloader="$$( $(UV) run python -m model.scripts.get_keyboard_metadata "$(KEYBOARD_JSON)" bootloader)" || exit 1; \
	mounted=false; \
	case "$$bootloader" in \
		rp2040) $(MAKE) _mount_uf2_volume || exit 1; mounted=true ;; \
	esac; \
	status=0; \
	$(QMK) flash -kb $(QMK_KEYBOARD) -km $(QMK_KEYMAP) $(QMK_FLAGS) || status=$$?; \
	if [ "$$mounted" = true ]; then \
		$(MAKE) _unmount_uf2_volume || [ "$$status" -ne 0 ] || status=$$?; \
	fi; \
	exit "$$status"

.PHONY: _mount_uf2_volume
_mount_uf2_volume:
	@echo "Waiting for the $(UF2_VOLUME_LABEL) volume; put the board into its bootloader now..."
	$(UV) run python -m firmware.tools.mount_uf2_volume --label "$(UF2_VOLUME_LABEL)" --sudo "$(SUDO)"

.PHONY: _unmount_uf2_volume
_unmount_uf2_volume:
	$(UV) run python -m firmware.tools.mount_uf2_volume --label "$(UF2_VOLUME_LABEL)" --sudo "$(SUDO)" --unmount

.PHONY: patch-load
patch-load:
ifdef KEYBOARD_ID
	@echo "Loading keyboard configuration from $(QMK_HOME)..."
	cp "$(QMK_HOME)/keyboards/$(QMK_KEYBOARD)/config.h" "$(KEYBOARDS_DIR)/$(KEYBOARD_ID)/config.h"
	cp "$(QMK_HOME)/keyboards/$(QMK_KEYBOARD)/keyboard.json" "$(KEYBOARDS_DIR)/$(KEYBOARD_ID)/keyboard.json"
	cp -r "$(QMK_HOME)/keyboards/$(QMK_KEYBOARD)/keymaps/$(QMK_KEYMAP)/." "$(KEYBOARDS_DIR)/$(KEYBOARD_ID)/keymap/"
	@echo "✔ Keyboard configuration loaded"
else
	+@$(call FOR_EACH_KEYBOARD,patching,Patching,patch-load)
endif

.PHONY: print-vars
print-vars:
	@echo "MISE=$(MISE)"
	@echo "QMK=$(QMK)"
	@echo "UV=$(UV)"
	@echo ""
	@echo "QMK_HOME=$(QMK_HOME)"
	@echo "QMK_KEYBOARD=$(QMK_KEYBOARD)"
	@echo "QMK_KEYMAP=$(QMK_KEYMAP)"
	@echo "KEYBOARD_JSON=$(KEYBOARD_JSON)"
	@echo "KEYBOARD_CONFIG=$(KEYBOARD_CONFIG)"
	@echo "QMK_KEYMAP_C=$(QMK_KEYMAP_C)"
	@echo "DEVICE_PID=$(DEVICE_PID)"
	@echo "LAYOUT_NAME=$(LAYOUT_NAME)"
	@echo "PIXELS_PER_UNIT=$(PIXELS_PER_UNIT)"
	@echo ""
	@echo "BUILD_DIR=$(BUILD_DIR)"
	@echo "ASSET_BUILD_DIR=$(ASSET_BUILD_DIR)"
	@echo "VIAL_JSON=$(VIAL_JSON)"
	@echo "CONSOLIDATED_ASSET=$(CONSOLIDATED_ASSET)"
	@echo "OVERLAY_PLATFORM=$(OVERLAY_PLATFORM)"
	@echo ""
	@echo "KEYMAP_OVERLAY_BIN_DIR=$(KEYMAP_OVERLAY_BIN_DIR)"
	@echo "KEYMAP_OVERLAY_BINARY=$(KEYMAP_OVERLAY_BINARY)"
	@echo "KEYBOARDS_DIR=$(KEYBOARDS_DIR)"

# ================= INTERNAL TARGETS =================

.PHONY: _internal_draw_layers
_internal_draw_layers: $(CONSOLIDATED_ASSET)

# ================= FILE RULES =================

$(BUILD_DIR):
	mkdir -p $(BUILD_DIR)

$(ASSET_BUILD_DIR):
	mkdir -p $(ASSET_BUILD_DIR)

# The installed overlay refreshes through this same library in-process. This
# development-only binary exposes it to the explicit draw-layers task;
# it is not copied into release archives or installed beside the overlay.
NATIVE_GENERATOR_DEPS := Cargo.toml Cargo.lock overlay/keymap-overlay-generator/Cargo.toml
NATIVE_GENERATOR_DEPS += $(wildcard overlay/keymap-overlay-generator/src/*.rs)

$(CONSOLIDATED_ASSET): $(KEYBOARD_JSON) $(KEYBOARD_CONFIG) $(NATIVE_GENERATOR_DEPS) | $(ASSET_BUILD_DIR)
	$(call WRITE_OUTPUT,$@,$(CARGO) run --quiet --package keymap-overlay-generator --bin keymap-overlay-generator -- --keyboard-json "$(KEYBOARD_JSON)" --keyboard-config "$(KEYBOARD_CONFIG)" --layout-name "$(LAYOUT_NAME)" --keyboard-id "$(KEYBOARD_ID)" --platform "$(OVERLAY_PLATFORM)" --pixels-per-unit "$(PIXELS_PER_UNIT)")

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

# Asset generation normally targets the current host. WSL must override this
# to `windows` because it generates files for the native Windows overlay.
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

# By default, `make install-assets` and `make draw-layers` load the keymap
# from the connected keyboard's VIAL EEPROM dump: it reflects live edits made
# in the Vial GUI, not just what keymap.c last compiled to. Set VIAL=false to
# render straight from keymap.c instead, with no device connected.
VIAL ?= true

# Set DRY_RUN=true to have flash-keymap resolve and print what it would write
# without touching the device's EEPROM.
DRY_RUN ?= false

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
UV ?= $(MISE) exec -- uv
CARGO ?= $(MISE) exec -- cargo
VITALY ?= $(MISE) exec cargo:vitaly@$(VITALY_VERSION) -- vitaly

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
# The installed service passes this to the overlay for startup self-heal
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

KEYMAP_PREFIX := $(KEYBOARD_ID)_

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

# Contains the full, unmodified keymap definition (layers, keycodes) in QMK format.
# Type: model/src/types.py:QmkKeymapJson
# Generated from: 'qmk c2json' (source) or 'generate_qmk_keymap_from_vitaly.py' (VIAL).
# Used by: the overlay asset generator, 'generate_vitaly_layout.py' (flashing).
QMK_KEYMAP_JSON := $(BUILD_DIR)/qmk-keymap.json

# Mapping of QMK hex keycodes to their string names (e.g., 0x0004 -> KC_A).
# Type: model/src/types.py:KeycodesJson
# Generated from: 'generate_keycodes.py' scanning QMK firmware.
# Used by: the overlay asset generator for name resolution.
KEYCODES_JSON := $(BUILD_DIR)/keycodes.json

# Mapping of user-defined enum keycodes (e.g., 0x7E40 -> SAFE_RANGE) from
# keymap.c, or from the device's embedded Vial definition under VIAL=true.
# Type: model/src/types.py:KeycodesJson
# Generated from: 'generate_custom_keycodes.py' parsing 'keymap.c' or $(VIAL_DEFINITION_JSON).
# Used by: the overlay asset generator and 'generate_vitaly_layout.py'.
CUSTOM_KEYCODES_JSON := $(BUILD_DIR)/custom-keycodes.json

# VIAL-compatible keyboard definition (matrix, layout, VID/PID, customKeycodes).
# Type: model/src/types.py:VialJson
# Generated from: 'generate_vial.py' using keyboard.json and keymap.c.
# Used by: 'qmk compile' (embedded in firmware) for VIAL support.
VIAL_JSON := $(BUILD_DIR)/vial.json

# Temporary dump of the keyboard's current VIAL configuration.
# Type: model/src/types.py:VitalyJson
# Generated from: 'vitaly save' (downloaded from device).
# Used by: 'generate_qmk_keymap_from_vitaly.py' (source for rebuild), 'generate_vitaly_layout.py' (base for merge).
VITALY_JSON := $(BUILD_DIR)/vitaly.json

# The connected device's own embedded Vial definition, decompressed. Custom
# keycode identity for VIAL=true rendering comes from here, not keymap.c, so
# deleting keymap.c after flashing does not break 'make install-assets VIAL=true'.
# Type: model/src/types.py:VialJson
# Generated from: 'fetch_vial_definition.py' (downloaded from device).
# Used by: $(CUSTOM_KEYCODES_JSON) and the overlay asset generator, under VIAL=true.
VIAL_DEFINITION_JSON := $(BUILD_DIR)/vial_definition.json

# Same lazy-and-cached treatment as DEVICE_PID. These are only meaningful once
# $(QMK_KEYMAP_JSON) exists, which is why install-assets/draw-layers build it in a
# first pass and then re-enter make to expand $(ASSETS).
LAYERS = $(eval LAYERS := $(shell if [ -s $(QMK_KEYMAP_JSON) ]; then $(UV) run python -m model.scripts.count_layers "$(QMK_KEYMAP_JSON)" || echo 0; else echo 0; fi))$(LAYERS)
ASSET_EXTENSION := json
STALE_ASSET_EXTENSION := png
ASSET_BUILD_DIR := $(BUILD_DIR)/assets/$(OVERLAY_PLATFORM)
ASSETS = $(eval ASSETS := $(shell if [ $(LAYERS) -gt 0 ]; then seq -f "$(ASSET_BUILD_DIR)/$(KEYMAP_PREFIX)L%g.$(ASSET_EXTENSION)" 0 $$(( $(LAYERS) - 1 )); fi))$(ASSETS)

# The per-layer files above stay a build-time intermediate; installing and
# reading them is one keyboard, one file, since a keyboard's layers are always
# generated and read together. Under VIAL=true they are not built at all: the
# native generator reads the device directly in one session (see FILE RULES).
CONSOLIDATED_ASSET := $(ASSET_BUILD_DIR)/$(KEYBOARD_ID).$(ASSET_EXTENSION)

endif

# ================= NATIVE ASSET GENERATOR =================
# Standalone Cargo workspace (see its Cargo.toml), not a root workspace
# member: vitaly wants hidapi's default features, while the platform overlay
# crates need a specific backend (Linux hidraw, not the libusb default, which
# detaches the kernel driver and stops the keyboard from typing). A shared
# Cargo.lock would unify those feature sets across every workspace member.
GENERATOR_MANIFEST := overlay/keymap-overlay-generator/Cargo.toml
GENERATOR_BINARY := overlay/keymap-overlay-generator/target/release/keymap-overlay-generator$(EXE_SUFFIX)
# Second [[bin]] in the same crate/build, built together by _build_generator.
FLASHER_BINARY := overlay/keymap-overlay-generator/target/release/keymap-overlay-flash-keymap$(EXE_SUFFIX)

.PHONY: _build_generator
_build_generator:
	$(CARGO) build --release --manifest-path "$(GENERATOR_MANIFEST)"

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
# Local rather than roaming %APPDATA%, because generated models and a log both
# describe one machine. Falls back the way the overlay itself does, so make and
# the binary always name the same directory.
WINDOWS_LOCAL_APP_DATA := $(shell cygpath -u "$$LOCALAPPDATA" 2>/dev/null)
ifeq ($(strip $(WINDOWS_LOCAL_APP_DATA)),)
WINDOWS_LOCAL_APP_DATA := $(WINDOWS_USER_HOME)/AppData/Local
endif
KEYMAP_OVERLAY_DIR ?= $(WINDOWS_LOCAL_APP_DATA)/keymap-overlay
KEYMAP_OVERLAY_LOG_DIR ?= $(WINDOWS_LOCAL_APP_DATA)/keymap-overlay/logs
# Where a per-user install puts an executable on Windows, the same place VS Code
# and Slack use. The Run key names it by absolute path, so it need not be on
# PATH.
KEYMAP_OVERLAY_BIN_DIR ?= $(WINDOWS_LOCAL_APP_DATA)/Programs/keymap-overlay
else
# The installed models are a regenerable cache of what a VIAL-flashed device
# already knows, not configuration, so they sit under .cache rather than
# .config.
KEYMAP_OVERLAY_DIR := $(HOME)/.cache/keymap-overlay
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
	@echo "      'make prepare-flash-keymap'; native Raw HID targets run here."

.PHONY: doctor
doctor:
	@set -o pipefail; \
	$(QMK) doctor -n 2>&1 | sed '/QMK home does not appear to be a Git repository! (no .git folder)/d'; \
	status=$${PIPESTATUS[0]}; \
	[ "$$status" -eq 0 ] || [ "$$status" -eq 1 ] || exit "$$status"

# This is no longer part of the normal source-install workflow: every native
# overlay owns its installed model directory through startup self-heal
# (`--keyboard-config-dir`). It remains useful for a manual force-refresh when
# fill-if-missing would not pick up a connected-device change. The default
# VIAL=true path is a native Raw HID read and therefore also runs on Windows.
#
# Under VIAL=false, LAYERS depends on $(QMK_KEYMAP_JSON), so install-assets
# and draw-layers build the QMK JSON in a first make invocation, then re-enter
# make to expand assets. Under VIAL=true the native generator (FILE RULES)
# determines its own layer count from the device, so this first pass — and
# the extra device read building $(QMK_KEYMAP_JSON) would cost — is skipped.
.PHONY: install-assets
install-assets:
ifdef KEYBOARD_ID
ifeq ($(VIAL),true)
	@$(MAKE) _internal_install
else
	@if [ "$(OS_FAMILY)" = "windows" ]; then \
		echo "ERROR: install-assets VIAL=false needs QMK source processing."; \
		echo "Run it from WSL, macOS or Linux; native Windows supports VIAL=true."; \
		exit 1; \
	fi
	@$(MAKE) $(QMK_KEYMAP_JSON)
	@$(MAKE) _internal_install
endif
else
	+@$(call FOR_EACH_KEYBOARD,installing,Installing,install-assets)
endif

# Kept as a compatibility alias for existing scripts. New callers should use
# install-assets.
.PHONY: install
install: install-assets

.PHONY: draw-layers
draw-layers:
ifdef KEYBOARD_ID
ifeq ($(VIAL),true)
	@$(MAKE) _internal_draw_layers
else
	@$(MAKE) $(QMK_KEYMAP_JSON)
	@$(MAKE) _internal_draw_layers
endif
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
test-wpf-e2e-windows: build-overlay _build_generator
ifeq ($(OS_FAMILY),windows)
	install -C "$(GENERATOR_BINARY)" "$(WPF_PUBLISH_DIR)/keymap-overlay-generator$(EXE_SUFFIX)"
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
	$(KEYMAP_OVERLAY) --asset-dir "$(KEYMAP_OVERLAY_DIR)" $(if $(SIMULATE),--simulate "$(SIMULATE)")

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

.PHONY: install-overlay
# Not install-assets: startup self-heal (FILE RULES, --keyboard-config-dir
# below) now generates any keyboard missing from $(KEYMAP_OVERLAY_DIR) itself
# once the service starts, so installing doesn't need to write there too.
# Run `make install-assets` by hand to force a refresh after changing the
# connected device through Vial or flash-keymap, since self-heal only fills in
# what's missing, not what's stale.
install-overlay: build-overlay
	@mkdir -p "$(KEYMAP_OVERLAY_DIR)" "$(KEYMAP_OVERLAY_BIN_DIR)" "$(KEYMAP_OVERLAY_LOG_DIR)"
# Windows holds an open executable locked, so the running overlay has to go
# before its binary can be replaced. The other two systems replace the file
# underneath the running process and stop it as part of installing the service.
	@$(MAKE) _stop_service_$(OS_FAMILY)
	install -C "$(OVERLAY_BUILD_BINARY)" "$(KEYMAP_OVERLAY_BINARY)"
# Installed alongside the frontend so self-heal (FILE RULES) can shell out to
# it by relative path at startup.
	@$(MAKE) _build_generator
	install -C "$(GENERATOR_BINARY)" "$(KEYMAP_OVERLAY_BIN_DIR)/keymap-overlay-generator$(EXE_SUFFIX)"
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
		'    <string>--asset-dir</string>' \
		'    <string>$(call xml_escape,$(KEYMAP_OVERLAY_DIR))</string>' \
		'    <string>--keyboard-config-dir</string>' \
		'    <string>$(call xml_escape,$(KEYBOARDS_DIR_ABS))</string>' \
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
		'ExecStart="$(KEYMAP_OVERLAY_BINARY)" --asset-dir "$(KEYMAP_OVERLAY_DIR)" --keyboard-config-dir "$(KEYBOARDS_DIR_ABS)"' \
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
	assets="$$(cygpath -w "$(KEYMAP_OVERLAY_DIR)")"; \
	configs="$$(cygpath -w "$(KEYBOARDS_DIR_ABS)")"; \
	env KEYMAP_OVERLAY_BINARY="$$binary" KEYMAP_OVERLAY_ASSETS="$$assets" KEYMAP_OVERLAY_CONFIGS="$$configs" MSYS2_ARG_CONV_EXCL='*' powershell.exe -NoProfile -NonInteractive -Command \
	'$$ErrorActionPreference = "Stop"; $$quote = [char]34; $$command = $$quote + $$env:KEYMAP_OVERLAY_BINARY + $$quote + " --asset-dir " + $$quote + $$env:KEYMAP_OVERLAY_ASSETS + $$quote + " --keyboard-config-dir " + $$quote + $$env:KEYMAP_OVERLAY_CONFIGS + $$quote; Set-ItemProperty -Path "HKCU:\Software\Microsoft\Windows\CurrentVersion\Run" -Name "$(KEYMAP_OVERLAY_RUN_VALUE)" -Value $$command; Start-Process -FilePath $$env:KEYMAP_OVERLAY_BINARY -ArgumentList "--asset-dir", ($$quote + $$env:KEYMAP_OVERLAY_ASSETS + $$quote), "--keyboard-config-dir", ($$quote + $$env:KEYMAP_OVERLAY_CONFIGS + $$quote)'

.PHONY: uninstall-overlay
uninstall-overlay:
	@$(MAKE) _uninstall_service_$(OS_FAMILY)
	rm -f "$(KEYMAP_OVERLAY_BINARY)"
	rm -f "$(KEYMAP_OVERLAY_BIN_DIR)/keymap-overlay-generator$(EXE_SUFFIX)"
	@$(MAKE) _uninstall_renderer_$(OS_FAMILY)
	rm -f "$(KEYMAP_OVERLAY_DIR)"/*.png
	rm -f "$(KEYMAP_OVERLAY_DIR)"/*.json
	@echo "✔ Overlay service and installed assets removed; logs remain at $(KEYMAP_OVERLAY_LOG_DIR)"

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
	$(call WRITE_OUTPUT,$(VIAL_JSON),$(UV) run python -m model.scripts.generate_vial --keyboard-json $(KEYBOARDS_DIR)/$(KEYBOARD_ID)/keyboard.json --layout-name "$(LAYOUT_NAME)" --keymap-c "$(QMK_KEYMAP_C)")
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
	@$(MAKE) compile
	@$(MAKE) _flash_$(OS_FAMILY)

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

.PHONY: flash-keymap
flash-keymap:
ifeq ($(OS_FAMILY),windows)
	@echo "ERROR: flash-keymap includes QMK source processing, which is not supported in MSYS2."; \
		echo "Run 'make prepare-flash-keymap KEYBOARD_ID=<id>' in WSL, then"; \
		echo "run 'make write-keymap KEYBOARD_ID=<id>' here for the native HID write."; \
		exit 1
else
# Only an explicit VIAL=true is an error here, not the plain default: VIAL=true
# would read the device and write it straight back, but flash-keymap always
# reads keymap.c regardless of the default, via the VIAL=false below.
ifeq ($(VIAL),true)
ifneq ($(origin VIAL),file)
	$(error flash-keymap writes keymap.c to the device; VIAL=true would read the device and write it straight back)
endif
endif
ifdef KEYBOARD_ID
	@$(MAKE) prepare-flash-keymap
	@$(MAKE) write-keymap
else
	+@$(call FOR_EACH_KEYBOARD,flashing,Flashing keymap for,flash-keymap)
endif
endif

.PHONY: prepare-flash-keymap
prepare-flash-keymap:
ifeq ($(OS_FAMILY),windows)
	$(error prepare-flash-keymap $(WINDOWS_FIRMWARE_ERROR))
endif
ifdef KEYBOARD_ID
	@$(MAKE) VIAL=false $(QMK_KEYMAP_JSON)
else
	+@$(call FOR_EACH_KEYBOARD,preparing,Preparing keymap for,prepare-flash-keymap)
endif

.PHONY: write-keymap
write-keymap:
ifdef KEYBOARD_ID
	@if [ ! -s "$(QMK_KEYMAP_JSON)" ]; then \
		echo "ERROR: prepared QMK keymap JSON is missing: $(QMK_KEYMAP_JSON)"; \
		echo "Run 'make prepare-flash-keymap KEYBOARD_ID=$(KEYBOARD_ID)' in WSL, macOS or Linux first."; \
		exit 1; \
	fi
	@$(MAKE) _build_generator
# The renderer resolves KC_TRNS only in memory; the flasher writes qmk
# c2json's output as-is, so EEPROM keeps the literal transparent marker and
# transparent-key inheritance keeps working. It writes the keymap and
# encoders directly (vitaly::protocol::set_keymap/set_encoder) — no
# read-current-state-and-merge round trip, since nothing else is touched.
ifeq ($(DRY_RUN),true)
	@echo "Resolving keymap.c for the device (dry run, nothing will be written)..."
else
	@echo "Writing keymap.c to the device..."
endif
	"$(FLASHER_BINARY)" --qmk-keymap-json "$(QMK_KEYMAP_JSON)" --keyboard-json "$(KEYBOARDS_DIR)/$(KEYBOARD_ID)/keyboard.json" --keymap-c "$(QMK_KEYMAP_C)" --layout-name "$(LAYOUT_NAME)" $(if $(filter true,$(DRY_RUN)),--dry-run)
else
	+@$(call FOR_EACH_KEYBOARD,writing,Writing keymap for,write-keymap)
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
	+@$(call FOR_EACH_KEYBOARD,patching,Patching,patch-load)
endif

.PHONY: print-vars
print-vars:
	@echo "VIAL=$(VIAL)"
	@echo ""
	@echo "MISE=$(MISE)"
	@echo "QMK=$(QMK)"
	@echo "UV=$(UV)"
	@echo "VITALY=$(VITALY)"
	@echo ""
	@echo "QMK_HOME=$(QMK_HOME)"
	@echo "QMK_KEYBOARD=$(QMK_KEYBOARD)"
	@echo "KEYMAP_PREFIX=$(KEYMAP_PREFIX)"
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
	@echo "QMK_KEYMAP_JSON=$(QMK_KEYMAP_JSON)"
	@echo "KEYCODES_JSON=$(KEYCODES_JSON)"
	@echo "CUSTOM_KEYCODES_JSON=$(CUSTOM_KEYCODES_JSON)"
	@echo "VIAL_JSON=$(VIAL_JSON)"
	@echo "VITALY_JSON=$(VITALY_JSON)"
	@echo "VIAL_DEFINITION_JSON=$(VIAL_DEFINITION_JSON)"
	@echo "LAYERS=$(LAYERS)"
	@echo "ASSETS=$(ASSETS)"
	@echo "CONSOLIDATED_ASSET=$(CONSOLIDATED_ASSET)"
	@echo "GENERATOR_BINARY=$(GENERATOR_BINARY)"
	@echo "OVERLAY_PLATFORM=$(OVERLAY_PLATFORM)"
	@echo ""
	@echo "KEYMAP_OVERLAY_DIR=$(KEYMAP_OVERLAY_DIR)"
	@echo "KEYMAP_OVERLAY_BIN_DIR=$(KEYMAP_OVERLAY_BIN_DIR)"
	@echo "KEYMAP_OVERLAY_BINARY=$(KEYMAP_OVERLAY_BINARY)"
	@echo "KEYBOARDS_DIR=$(KEYBOARDS_DIR)"

# ================= INTERNAL TARGETS =================

.PHONY: _internal_install
_internal_install: $(CONSOLIDATED_ASSET)
# Under VIAL=true the generator already refuses to produce a zero-layer file
# (Make would have stopped above), so $(LAYERS) — which needs
# $(QMK_KEYMAP_JSON), deliberately not built on this path — is skipped here.
	@if [ "$(VIAL)" != "true" ] && [ "$(LAYERS)" -eq "0" ]; then \
		echo "ERROR: No layers found even after generation."; \
		exit 1; \
	fi
	@echo "Installing keymap overlay assets..."
	@mkdir -p "$(KEYMAP_OVERLAY_DIR)"
	@for asset in "$(ASSET_BUILD_DIR)"/$(KEYMAP_PREFIX)L*.$(ASSET_EXTENSION); do \
		case " $(ASSETS) " in *" $$asset "*) ;; *) rm -f "$$asset" ;; esac; \
		done
	@cp "$(CONSOLIDATED_ASSET)" "$(KEYMAP_OVERLAY_DIR)/$(KEYBOARD_ID).$(ASSET_EXTENSION)"
# Stale leftovers from the previous one-file-per-layer format, if any.
	@rm -f "$(KEYMAP_OVERLAY_DIR)"/$(KEYMAP_PREFIX)L*.$(ASSET_EXTENSION)
	@rm -f "$(KEYMAP_OVERLAY_DIR)"/$(KEYMAP_PREFIX)L*.$(STALE_ASSET_EXTENSION)
	@echo "✔ Overlay assets installed; run 'make run-overlay' to start the native app"

.PHONY: _internal_draw_layers
_internal_draw_layers: $(CONSOLIDATED_ASSET)

# ================= FILE RULES =================

$(BUILD_DIR):
	mkdir -p $(BUILD_DIR)

$(ASSET_BUILD_DIR):
	mkdir -p $(ASSET_BUILD_DIR)

RENDER_ASSET_DEPS := $(QMK_KEYMAP_JSON) $(KEYBOARD_JSON) $(KEYBOARD_CONFIG) $(CUSTOM_KEYCODES_JSON) model/scripts/encoder_map.py model/scripts/generate_overlay_asset.py model/src/types.py model/src/util.py
ifeq ($(VIAL),true)
# Sourced from the connected device's own embedded Vial definition, not
# keymap.c, so deleting keymap.c after flashing does not break rendering.
RENDER_ASSET_DEPS += $(VIAL_DEFINITION_JSON)
RENDER_ENCODER_INPUT := --vitaly-json "$(VITALY_JSON)" --vial-definition-json "$(VIAL_DEFINITION_JSON)"
else
RENDER_ASSET_DEPS += $(QMK_KEYMAP_C)
RENDER_ENCODER_INPUT := --keymap-c "$(QMK_KEYMAP_C)"
endif

$(ASSET_BUILD_DIR)/$(KEYMAP_PREFIX)L%.$(ASSET_EXTENSION): $(RENDER_ASSET_DEPS) | $(ASSET_BUILD_DIR)
	$(call WRITE_OUTPUT,$@,$(UV) run python -m model.scripts.generate_overlay_asset --qmk-keymap-json "$(QMK_KEYMAP_JSON)" --keyboard-json "$(KEYBOARD_JSON)" --keyboard-config "$(KEYBOARD_CONFIG)" --custom-keycodes-json "$(CUSTOM_KEYCODES_JSON)" --layout-name "$(LAYOUT_NAME)" --layer "$*" --pixels-per-unit "$(PIXELS_PER_UNIT)" --platform "$(OVERLAY_PLATFORM)" $(RENDER_ENCODER_INPUT))

ifeq ($(VIAL),true)
# Reads the connected device directly in one HID session (customKeycodes,
# every layer, every encoder) and renders straight to the consolidated file;
# there is no per-layer file or separate consolidation step on this path.
$(CONSOLIDATED_ASSET): _force_build $(KEYBOARD_JSON) $(KEYBOARD_CONFIG) $(shell find overlay/keymap-overlay-generator/src -name '*.rs') | $(ASSET_BUILD_DIR)
	@$(MAKE) _build_generator
	$(call WRITE_OUTPUT,$@,"$(GENERATOR_BINARY)" --keyboard-json "$(KEYBOARD_JSON)" --keyboard-config "$(KEYBOARD_CONFIG)" --layout-name "$(LAYOUT_NAME)" --keyboard-id "$(KEYBOARD_ID)" --platform "$(OVERLAY_PLATFORM)" --pixels-per-unit "$(PIXELS_PER_UNIT)")
else
# Passed the exact current $(ASSETS) paths, not a directory to glob, so a
# leftover from a shrunk layer count can never sneak into the installed file.
$(CONSOLIDATED_ASSET): $(ASSETS) model/scripts/consolidate_layer_models.py | $(ASSET_BUILD_DIR)
	$(call WRITE_OUTPUT,$@,$(UV) run python -m model.scripts.consolidate_layer_models --keyboard-id "$(KEYBOARD_ID)" $(foreach asset,$(ASSETS),--layer-json "$(asset)"))
endif

.PHONY: _force_build
_force_build:

# c2json reads the keymap and resolves its layout against the keyboard
# definition, so both are inputs, as is anything keymap.c includes from beside
# it. _copy_firmware installs config.h and layer_notify.h too, but those only
# reach the compiler, never this JSON, so listing them would rebuild the assets
# for changes that cannot alter them. sort also dedupes keymap.c out of the
# wildcard; the explicit entry stays so a missing keymap.c is still an error.
QMK_KEYMAP_JSON_DEPS := $(sort $(QMK_KEYMAP_C) $(KEYBOARD_JSON) \
	$(wildcard $(KEYBOARDS_DIR)/$(KEYBOARD_ID)/keymap/*))
QMK_KEYMAP_JSON_ORDER_DEPS := $(BUILD_DIR)
ifeq ($(VIAL),true)
QMK_KEYMAP_JSON_DEPS += _force_build
else
# The example's keyboard definition and keymap live outside QMK's vendored
# tree. Install them before c2json validates -kb, including on a fresh clone.
#
# Order-only, unlike _force_build above, which is phony on purpose. A phony
# normal prerequisite is always out of date, so it would remake the raw JSON on
# every invocation and cascade through every asset, re-running the renderer per
# layer per keyboard with nothing changed.
# This only has to have run before c2json validates -kb, which is what
# order-only means. The files it copies are tracked above, as themselves.
QMK_KEYMAP_JSON_ORDER_DEPS += _copy_firmware
endif

$(QMK_KEYMAP_JSON): $(QMK_KEYMAP_JSON_DEPS) | $(QMK_KEYMAP_JSON_ORDER_DEPS)
ifeq ($(VIAL),true)
	@echo "Dumping QMK JSON from VIAL EEPROM..."
	$(VITALY) -i $(DEVICE_PID) save -f $(VITALY_JSON)
	@[ -s "$(VITALY_JSON)" ] || (echo "ERROR: No VIAL dump found at $(VITALY_JSON)"; exit 1)
	$(call WRITE_OUTPUT,$@,$(UV) run python -m model.scripts.generate_qmk_keymap_from_vitaly --vitaly-json $(VITALY_JSON) --keyboard-json "$(KEYBOARD_JSON)" --layout-name "$(LAYOUT_NAME)")
else
	@echo "Compiling QMK JSON from source..."
	$(call WRITE_OUTPUT,$@,$(QMK) c2json --no-cpp -kb $(QMK_KEYBOARD) -km $(QMK_KEYMAP) "$(QMK_KEYMAP_C)")
endif

$(KEYCODES_JSON): model/scripts/generate_keycodes.py | $(BUILD_DIR)
	$(call WRITE_OUTPUT,$@,$(UV) run python -m model.scripts.generate_keycodes --qmk-dir "$(QMK_HOME)")

ifeq ($(VIAL),true)
# Re-fetched from the device on every build, same as $(VITALY_JSON) above.
$(VIAL_DEFINITION_JSON): _force_build model/scripts/fetch_vial_definition.py | $(BUILD_DIR)
	$(call WRITE_OUTPUT,$@,$(UV) run python -m model.scripts.fetch_vial_definition --keyboard-json "$(KEYBOARD_JSON)")

$(CUSTOM_KEYCODES_JSON): $(VIAL_DEFINITION_JSON) model/scripts/generate_custom_keycodes.py | $(BUILD_DIR)
	$(call WRITE_OUTPUT,$@,$(UV) run python -m model.scripts.generate_custom_keycodes --vial-definition-json "$(VIAL_DEFINITION_JSON)")
else
$(CUSTOM_KEYCODES_JSON): $(QMK_KEYMAP_C) model/scripts/generate_custom_keycodes.py $(KEYCODES_JSON) | $(BUILD_DIR)
	$(call WRITE_OUTPUT,$@,$(UV) run python -m model.scripts.generate_custom_keycodes --keymap-c "$(QMK_KEYMAP_C)" --keycodes-json "$(KEYCODES_JSON)")
endif

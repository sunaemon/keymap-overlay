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
CARGO_LLVM_COV ?= $(MISE_DEV) exec -- cargo llvm-cov
LEFTHOOK ?= $(MISE) exec -- lefthook
QMK ?= $(QMK_ENV) $(MISE) exec -- qmk
UV ?= $(MISE) exec -- uv
CARGO ?= $(MISE) exec -- cargo

DISPLAY_MODEL_CONTRACT_DIR := overlay/display-model-contract
DISPLAY_MODEL_SCHEMA := $(DISPLAY_MODEL_CONTRACT_DIR)/display-model.schema.json
DISPLAY_MODEL_SCHEMA_COMMAND := $(CARGO) run --quiet --package keymap-overlay-generator --features contract-schema --bin generate-display-model-schema
CONTRACT_COVERAGE_IGNORE := /(custom_keycodes|device|labels|lib|model|qmk_keymap|vial)\.rs$$

QMK_TOOLCHAIN_PACKAGES := osx-cross/arm/arm-none-eabi-gcc@8 osx-cross/avr/avr-gcc@9 avrdude dfu-programmer dfu-util

# The same set per distribution, plus libudev for Raw HID and the Qt 6 /
# LayerShellQt stack used by the native KDE Plasma overlay.
LINUX_TOOLCHAIN_PACKAGES_PACMAN := arm-none-eabi-gcc arm-none-eabi-binutils arm-none-eabi-newlib avr-gcc avr-libc avrdude dfu-programmer dfu-util systemd-libs at-spi2-core xz cmake qt6-base qt6-declarative layer-shell-qt ttf-liberation
LINUX_TOOLCHAIN_PACKAGES_APT := gcc-arm-none-eabi binutils-arm-none-eabi libnewlib-arm-none-eabi gcc-avr avr-libc avrdude dfu-programmer dfu-util libudev-dev libatspi2.0-dev liblzma-dev cmake qt6-base-dev qt6-declarative-dev qt6-wayland qml6-module-qtqml-workerscript qml6-module-qtquick qml6-module-qtquick-window fonts-liberation
LINUX_LAYERSHELL_QML_APT := qml6-module-org-kde-layershell
LINUX_TOOLCHAIN_PACKAGES_DNF := arm-none-eabi-gcc-cs arm-none-eabi-newlib avr-gcc avr-libc avrdude dfu-programmer dfu-util systemd-devel at-spi2-core-devel xz-devel cmake qt6-qtbase-devel qt6-qtdeclarative-devel qt6-qtwayland layer-shell-qt liberation-mono-fonts

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

# launchd can briefly reject a bootstrap after bootout has returned while the
# old job is still being dismantled. Retry only this registration boundary;
# persistent errors still fail the install with launchctl's final message.
define BOOTSTRAP_KEYMAP_OVERLAY
if launchctl bootstrap "gui/$$(id -u)" "$(KEYMAP_OVERLAY_PLIST)"; then \
	:; \
	else \
	printf '%s\n' "launchctl bootstrap failed; retrying..." >&2; \
	sleep 1; \
	if launchctl bootstrap "gui/$$(id -u)" "$(KEYMAP_OVERLAY_PLIST)"; then \
	:; \
	else \
	printf '%s\n' "launchctl bootstrap failed; retrying..." >&2; \
	sleep 1; \
	launchctl bootstrap "gui/$$(id -u)" "$(KEYMAP_OVERLAY_PLIST)"; \
	fi; \
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
# Include every command-line value that changes the generated model in its
# output identity, so make cannot reuse a model generated at another scale.
ASSET_BUILD_DIR := $(BUILD_DIR)/assets/$(OVERLAY_PLATFORM)/$(PIXELS_PER_UNIT)
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

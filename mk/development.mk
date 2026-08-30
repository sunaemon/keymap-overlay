# ================= TARGETS =================

.PHONY: all
all: draw-layers

.PHONY: format
format:
	$(MISE_DEV) run format

.PHONY: generate-contracts
generate-contracts:
	$(call WRITE_OUTPUT,$(DISPLAY_MODEL_SCHEMA),$(DISPLAY_MODEL_SCHEMA_COMMAND))

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

.PHONY: clean
clean:
	rm -rf build

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
NATIVE_GENERATOR_DEPS += overlay/keymap-core/Cargo.toml
NATIVE_GENERATOR_DEPS += $(wildcard overlay/keymap-core/src/*.rs)

$(CONSOLIDATED_ASSET): $(KEYBOARD_JSON) $(KEYBOARD_CONFIG) $(NATIVE_GENERATOR_DEPS) | $(ASSET_BUILD_DIR)
	$(call WRITE_OUTPUT,$@,$(CARGO) run --quiet --package keymap-overlay-generator --bin keymap-overlay-generator -- --keyboard-json "$(KEYBOARD_JSON)" --keyboard-config "$(KEYBOARD_CONFIG)" --layout-name "$(LAYOUT_NAME)" --keyboard-id "$(KEYBOARD_ID)" --platform "$(OVERLAY_PLATFORM)" --pixels-per-unit "$(PIXELS_PER_UNIT)")

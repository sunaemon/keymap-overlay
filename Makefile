SHELL := /bin/bash

# ================= VIA CONFIGURATION =================

# If VIAL is enabled, the keymap will load from the VIAL EEPROM dump in `make install` and `make draw-layers`.
# If VIAL is disabled, the keymap will be compiled from the firmware source.
VIAL ?= false

# ================= TOOLS CONFIGURATION =================
RSVG ?= $(MISE) exec -- resvg
MISE ?= mise
VITALY_VERSION := $(shell awk -F' *= *' '/^VITALY_VERSION *=/ {gsub(/"/,"",$$2); print $$2}' mise.toml)
ifeq ($(strip $(VITALY_VERSION)),)
$(error VITALY_VERSION is missing from mise.toml)
endif
QMK ?= $(MISE) exec -- qmk
KEYMAP ?= $(MISE) exec -- keymap
UV ?= $(MISE) exec -- uv
VITALY ?= $(MISE) exec cargo:vitaly@$(VITALY_VERSION) -- vitaly
RUN_OUTPUT := $(UV) run python -m scripts.run_output

# ================= QMK CONFIGURATION =================
QMK_HOME := qmk_firmware
export QMK_HOME := $(QMK_HOME)

QMK_KEYMAP ?= keymap

# Directory containing keyboard configurations
KEYBOARDS_DIR ?= example

ifdef KEYBOARD_ID

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

LAYERS := $(shell if [ -s $(QMK_KEYMAP_JSON) ]; then $(UV) run python -m scripts.count_layers "$(QMK_KEYMAP_JSON)" || echo 0; else echo 0; fi)
PNG := $(shell if [ $(LAYERS) -gt 0 ]; then seq -f "$(BUILD_DIR)/$(KEYMAP_PREFIX)L%g.png" 0 $$(( $(LAYERS) - 1 )); fi)

endif

# ================= HAMMERSPOON CONFIGURATION =================
HAMMERSPOON_DIR := $(HOME)/.hammerspoon
HAMMERSPOON_INIT := $(HAMMERSPOON_DIR)/init.lua
HAMMERSPOON_OVERLAY := $(HAMMERSPOON_DIR)/keymap-overlay.lua

# ================= TARGETS =================

.PHONY: all
all: draw-layers

.PHONY: format
format:
	$(MISE) run format

.PHONY: doctor
doctor:
	$(QMK) doctor

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
	$(MISE) run lint

.PHONY: test
test:
	$(UV) run pytest

.PHONY: clean
clean:
	rm -rf build

.PHONY: _copy_firmware
_copy_firmware:
ifndef KEYBOARD_ID
	$(error KEYBOARD_ID is required for _copy_firmware)
endif
	mkdir -p "$(QMK_HOME)/keyboards/$(QMK_KEYBOARD)/keymaps/$(QMK_KEYMAP)"
	mkdir -p "$(BUILD_DIR)"
	install -C $(KEYBOARDS_DIR)/$(KEYBOARD_ID)/config.h "$(QMK_HOME)/keyboards/$(QMK_KEYBOARD)/config.h"
	install -C $(KEYBOARDS_DIR)/$(KEYBOARD_ID)/keyboard.json "$(QMK_HOME)/keyboards/$(QMK_KEYBOARD)/keyboard.json"
	$(RUN_OUTPUT) "$(VIAL_JSON)" -- $(UV) run python -m scripts.generate_vial --keyboard-json $(KEYBOARDS_DIR)/$(KEYBOARD_ID)/keyboard.json --layout-name "$(LAYOUT_NAME)"
	install -C $(VIAL_JSON) "$(QMK_HOME)/keyboards/$(QMK_KEYBOARD)/keymaps/$(QMK_KEYMAP)/vial.json"
	install -C $(KEYBOARDS_DIR)/$(KEYBOARD_ID)/keymap/* "$(QMK_HOME)/keyboards/$(QMK_KEYBOARD)/keymaps/$(QMK_KEYMAP)/"

.PHONY: compile
compile:
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
ifndef KEYBOARD_ID
	$(error KEYBOARD_ID is required for flash)
endif
	@$(MAKE) compile
	$(QMK) flash -kb $(QMK_KEYBOARD) -km $(QMK_KEYMAP) $(QMK_FLAGS)

.PHONY: flash-keymap
flash-keymap:
ifdef KEYBOARD_ID
	@$(MAKE) $(QMK_KEYMAP_JSON) $(CUSTOM_KEYCODES_JSON)
	@echo "Fetching current configuration from device..."
	$(VITALY) save -f $(VITALY_JSON)
	@[ -s "$(VITALY_JSON)" ] || (echo "ERROR: No VIAL dump found at $(VITALY_JSON)"; exit 1)
	@echo "Merging QMK keymap into Vitaly configuration..."
	$(RUN_OUTPUT) "$(BUILD_DIR)/vitaly_ready.json" -- \
		$(UV) run python -m scripts.generate_vitaly_layout \
		--qmk-keymap-json "$(QMK_KEYMAP_JSON)" \
		--vitaly-json "$(VITALY_JSON)" \
		--keyboard-json "$(KEYBOARDS_DIR)/$(KEYBOARD_ID)/keyboard.json" \
		--custom-keycodes-json "$(CUSTOM_KEYCODES_JSON)" \
		--layout-name "$(LAYOUT_NAME)"
	@echo "Loading new configuration to device..."
	$(VITALY) load -f $(BUILD_DIR)/vitaly_ready.json
else
	@echo "KEYBOARD_ID not set, flashing keymap for all keyboards..."
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
	@echo "HAMMERSPOON_DIR=$(HAMMERSPOON_DIR)"
	@echo "HAMMERSPOON_INIT=$(HAMMERSPOON_INIT)"
	@echo "HAMMERSPOON_OVERLAY=$(HAMMERSPOON_OVERLAY)"
	@echo "KEYBOARDS_DIR=$(KEYBOARDS_DIR)"

# ================= INTERNAL TARGETS =================

.PHONY: _internal_install
_internal_install: keymap-overlay.lua $(PNG)
	@if [ "$(LAYERS)" -eq "0" ]; then \
		echo "ERROR: No layers found even after generation."; \
		exit 1; \
	fi
	@echo "Installing Hammerspoon keymap overlay..."
	@mkdir -p "$(HAMMERSPOON_DIR)"

	@echo "→ Copying keymap-overlay.lua"
	@cp keymap-overlay.lua "$(HAMMERSPOON_OVERLAY)"

	@echo "→ Copying keymap images ($(KEYMAP_PREFIX)L*.png)"
	@cp $(BUILD_DIR)/$(KEYMAP_PREFIX)L*.png "$(HAMMERSPOON_DIR)/"

	@touch "$(HAMMERSPOON_INIT)"

	@grep -q 'require("keymap-overlay")' "$(HAMMERSPOON_INIT)" || \
	  (echo "" >> "$(HAMMERSPOON_INIT)" && \
	  echo 'require("keymap-overlay")' >> "$(HAMMERSPOON_INIT)" && \
	  echo "→ Added require(\"keymap-overlay\") to init.lua")

	@echo "✔ Hammerspoon overlay installed"

.PHONY: _internal_draw_layers
_internal_draw_layers: $(PNG)

# ================= FILE RULES =================

$(BUILD_DIR):
	mkdir -p $(BUILD_DIR)

$(BUILD_DIR)/%.png: $(BUILD_DIR)/%.svg
	$(RSVG) --dpi $(DPI) "$<" "$@"

$(BUILD_DIR)/$(KEYMAP_PREFIX)L%.svg: $(KEYMAP_DRAWER_YAML) | $(BUILD_DIR)
	$(RUN_OUTPUT) "$@" -- $(KEYMAP) draw "$(KEYMAP_DRAWER_YAML)" -j "$(KEYBOARD_JSON)" -l "$(LAYOUT_NAME)" -s "L$*"

$(KEYMAP_DRAWER_YAML): $(QMK_KEYMAP_JSON) | $(BUILD_DIR)
	$(RUN_OUTPUT) "$@" -- $(KEYMAP) parse -q $(QMK_KEYMAP_JSON)

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
	$(VITALY) save -f $(VITALY_JSON)
	@[ -s "$(VITALY_JSON)" ] || (echo "ERROR: No VIAL dump found at $(VITALY_JSON)"; exit 1)
	$(RUN_OUTPUT) "$@" -- $(UV) run python -m scripts.generate_qmk_keymap_from_vitaly --vitaly-json $(VITALY_JSON) --keyboard-json "$(KEYBOARD_JSON)" --layout-name "$(LAYOUT_NAME)"
else
	@echo "Compiling QMK JSON from source..."
	$(RUN_OUTPUT) "$@" -- $(QMK) c2json --no-cpp -kb $(QMK_KEYBOARD) -km $(QMK_KEYMAP) "$(QMK_KEYMAP_C)"
endif

$(QMK_KEYMAP_JSON): $(QMK_KEYMAP_JSON_DEPS) | $(BUILD_DIR)
	$(RUN_OUTPUT) "$@" -- $(UV) run python -m scripts.postprocess_qmk_keymap "$(QMK_KEYMAP_JSON_RAW)" --custom-keycodes-json $(CUSTOM_KEYCODES_JSON)

$(KEYCODES_JSON): scripts/generate_keycodes.py | $(BUILD_DIR)
	$(RUN_OUTPUT) "$@" -- $(UV) run python -m scripts.generate_keycodes --qmk-dir "$(QMK_HOME)"

$(CUSTOM_KEYCODES_JSON): $(QMK_KEYMAP_C) scripts/generate_custom_keycodes.py $(KEYCODES_JSON) | $(BUILD_DIR)
	$(RUN_OUTPUT) "$@" -- $(UV) run python -m scripts.generate_custom_keycodes "$(QMK_KEYMAP_C)" --keycodes-json "$(KEYCODES_JSON)"

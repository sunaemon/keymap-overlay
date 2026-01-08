SHELL := /bin/bash

# ================= VIA CONFIGURATION =================

# If VIAL is enabled, the keymap will load from the VIAL EEPROM dump in `make install` and `make draw-layers`.
# If VIAL is disabled, the keymap will be compiled from the firmware source.
VIAL ?= false

# ================= TOOLS CONFIGURATION =================
RSVG ?= rsvg-convert
MISE ?= mise
VITALY_VERSION := $(shell awk -F' *= *' '/^VITALY_VERSION *=/ {gsub(/"/,"",$$2); print $$2}' mise.toml)
ifeq ($(strip $(VITALY_VERSION)),)
$(error VITALY_VERSION is missing from mise.toml)
endif
QMK ?= qmk
KEYMAP ?= $(MISE) exec -- keymap
UV ?= $(MISE) exec -- uv
VITALY ?= $(MISE) exec cargo:vitaly@$(VITALY_VERSION) -- vitaly

# ================= QMK CONFIGURATION =================
QMK_HOME := $(CURDIR)/qmk_firmware
export QMK_HOME := $(QMK_HOME)

QMK_KEYBOARD := salicylic_acid3/insixty_en
QMK_KEYMAP := layer-notify

# [QMK Keyboard JSON]
# QMK keyboard definition (matrix/layouts/metadata).
# Type: src/types.py:KeyboardJson
KEYBOARD_JSON := $(QMK_HOME)/keyboards/$(QMK_KEYBOARD)/keyboard.json
QMK_KEYMAP_C := keyboards/$(QMK_KEYBOARD)/keymaps/$(QMK_KEYMAP)/keymap.c

LAYOUT_NAME := LAYOUT

ROWS ?= 5
COLUMNS ?= 14
DPI ?= 144

# ================= BUILD CONFIGURATION =================
BUILD_DIR := build

# [QMK Keymap JSON]
# Contains the full keymap definition (layers, keycodes) in QMK format.
# Type: src/types.py:QmkKeymapJson
# Generated from: 'qmk c2json' (source) or 'generate_qmk_keymap_from_vitaly.py' (VIAL).
# Used by: 'keymap-drawer' (visuals), 'generate_vitaly_layout.py' (flashing).
QMK_KEYMAP_JSON := $(BUILD_DIR)/qmk-keymap.json

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
# Used by: 'generate_qmk_keymap_postprocess.py' for name resolution.
KEYCODES_JSON := $(BUILD_DIR)/keycodes.json

# [Custom Keycodes JSON]
# Mapping of user-defined enum keycodes (e.g., 0x7E40 -> SAFE_RANGE) from keymap.c.
# Type: src/types.py:KeycodesJson
# Generated from: 'generate_custom_keycodes.py' parsing 'keymap.c'.
# Used by: 'generate_qmk_keymap_postprocess.py', 'generate_vitaly_layout.py' to preserve custom codes.
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

# [Key to Layer JSON]
# Mapping of F-keys to layer names (e.g., {"f13": "L1", "f14": "L2"}).
# Type: src/types.py:KeyToLayerJson
# Generated from: 'generate_key_to_layer.py' parsing 'keymap.c'.
# Used by: Hammerspoon overlay (lua) to know which image to show for which trigger key.
KEY_TO_LAYER_JSON := $(BUILD_DIR)/key-to-layer.json

LAYERS := $(shell if [ -s $(QMK_KEYMAP_JSON) ]; then $(UV) run scripts/count_layers.py "$(QMK_KEYMAP_JSON)" || echo 0; else echo 0; fi)
PNG := $(shell if [ $(LAYERS) -gt 0 ]; then seq -f "$(BUILD_DIR)/L%g.png" 0 $$(( $(LAYERS) - 1 )); fi)

# ================= HAMMERSPOON CONFIGURATION =================
HAMMERSPOON_DIR := $(HOME)/.hammerspoon
HAMMERSPOON_INIT := $(HAMMERSPOON_DIR)/init.lua
HAMMERSPOON_OVERLAY := $(HAMMERSPOON_DIR)/keymap-overlay.lua

# ================= TARGETS =================

.PHONY: all
all: draw-layers

.PHONY: setup
setup:
	brew install mise qmk librsvg
	mise install
	brew install --cask hammerspoon
	$(UV) sync --no-dev

.PHONY: setup-dev
setup-dev: setup
	$(UV) sync

.PHONY: format
format:
	$(MISE) run format

.PHONY: doctor
doctor:
	$(QMK) doctor

# Because  LAYERS variable depends on $(QMK_KEYMAP_JSON), we need to call draw-layers with another make invocation
.PHONY: install
install: $(QMK_KEYMAP_JSON)
	@$(MAKE) _internal_install

.PHONY: draw-layers
draw-layers: $(QMK_KEYMAP_JSON)
	@$(MAKE) _internal_draw_layers

.PHONY: lint
lint:
	$(MISE) run lint

.PHONY: test
test:
	$(UV) run pytest

.PHONY: clean
clean:
	rm -rf $(BUILD_DIR)

.PHONY: _copy_firmware
_copy_firmware:
	mkdir -p "$(QMK_HOME)/keyboards/$(QMK_KEYBOARD)/keymaps/$(QMK_KEYMAP)"
	mkdir -p "$(BUILD_DIR)"
	install -C keyboards/$(QMK_KEYBOARD)/config.h "$(QMK_HOME)/keyboards/$(QMK_KEYBOARD)/config.h"
	install -C keyboards/$(QMK_KEYBOARD)/keyboard.json "$(QMK_HOME)/keyboards/$(QMK_KEYBOARD)/keyboard.json"
	$(UV) run scripts/generate_vial.py --keyboard-json keyboards/$(QMK_KEYBOARD)/keyboard.json --layout-name "$(LAYOUT_NAME)" > "$(VIAL_JSON).tmp" || (rm -f "$(VIAL_JSON).tmp" && exit 1)
	mv "$(VIAL_JSON).tmp" "$(VIAL_JSON)"
	install -C $(VIAL_JSON) "$(QMK_HOME)/keyboards/$(QMK_KEYBOARD)/keymaps/$(QMK_KEYMAP)/vial.json"
	install -C keyboards/$(QMK_KEYBOARD)/keymaps/$(QMK_KEYMAP)/* "$(QMK_HOME)/keyboards/$(QMK_KEYBOARD)/keymaps/$(QMK_KEYMAP)/"

.PHONY: compile
compile: _copy_firmware
	qmk compile -kb $(QMK_KEYBOARD) -km $(QMK_KEYMAP)

.PHONY: flash
flash: compile
	qmk flash -kb $(QMK_KEYBOARD) -km $(QMK_KEYMAP)

.PHONY: flash-keymap
flash-keymap: $(QMK_KEYMAP_JSON) $(CUSTOM_KEYCODES_JSON)
	@echo "Fetching current configuration from device..."
	$(VITALY) save -f $(VITALY_JSON)
	@[ -s "$(VITALY_JSON)" ] || (echo "ERROR: No VIAL dump found at $(VITALY_JSON)"; exit 1)
	@echo "Merging QMK keymap into Vitaly configuration..."
	$(UV) run scripts/generate_vitaly_layout.py \
		--qmk-keymap-json "$(QMK_KEYMAP_JSON)" \
		--vitaly-json "$(VITALY_JSON)" \
		--keyboard-json "keyboards/$(QMK_KEYBOARD)/keyboard.json" \
		--custom-keycodes-json "$(CUSTOM_KEYCODES_JSON)" \
		--layout-name "$(LAYOUT_NAME)" \
		> "$(BUILD_DIR)/vitaly_ready.json.tmp" || (rm -f "$(BUILD_DIR)/vitaly_ready.json.tmp" && exit 1)
	mv "$(BUILD_DIR)/vitaly_ready.json.tmp" "$(BUILD_DIR)/vitaly_ready.json"
	@echo "Loading new configuration to device..."
	$(VITALY) load -f $(BUILD_DIR)/vitaly_ready.json

.PHONY: patch-load
patch-load:
	@echo "Loading keyboard configuration from $(QMK_HOME)..."
	cp "$(QMK_HOME)/keyboards/$(QMK_KEYBOARD)/config.h" "keyboards/$(QMK_KEYBOARD)/config.h"
	cp "$(QMK_HOME)/keyboards/$(QMK_KEYBOARD)/keyboard.json" "keyboards/$(QMK_KEYBOARD)/keyboard.json"
	cp -r "$(QMK_HOME)/keyboards/$(QMK_KEYBOARD)/keymaps/$(QMK_KEYMAP)" "keyboards/$(QMK_KEYBOARD)/keymaps/"
	@echo "✔ Keyboard configuration loaded"

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
	@echo "QMK_KEYMAP=$(QMK_KEYMAP)"
	@echo "KEYBOARD_JSON=$(KEYBOARD_JSON)"
	@echo "QMK_KEYMAP_C=$(QMK_KEYMAP_C)"
	@echo "LAYOUT_NAME=$(LAYOUT_NAME)"
	@echo "ROWS=$(ROWS)"
	@echo "COLUMNS=$(COLUMNS)"
	@echo "DPI=$(DPI)"
	@echo ""
	@echo "BUILD_DIR=$(BUILD_DIR)"
	@echo "QMK_KEYMAP_JSON=$(QMK_KEYMAP_JSON)"
	@echo "KEYMAP_DRAWER_YAML=$(KEYMAP_DRAWER_YAML)"
	@echo "KEYCODES_JSON=$(KEYCODES_JSON)"
	@echo "CUSTOM_KEYCODES_JSON=$(CUSTOM_KEYCODES_JSON)"
	@echo "VIAL_JSON=$(VIAL_JSON)"
	@echo "VITALY_JSON=$(VITALY_JSON)"
	@echo "KEY_TO_LAYER_JSON=$(KEY_TO_LAYER_JSON)"
	@echo "LAYERS=$(LAYERS)"
	@echo "PNG=$(PNG)"
	@echo ""
	@echo "HAMMERSPOON_DIR=$(HAMMERSPOON_DIR)"
	@echo "HAMMERSPOON_INIT=$(HAMMERSPOON_INIT)"
	@echo "HAMMERSPOON_OVERLAY=$(HAMMERSPOON_OVERLAY)"

# ================= INTERNAL TARGETS =================

.PHONY: _internal_install
_internal_install: keymap-overlay.lua $(PNG) $(KEY_TO_LAYER_JSON)
	@if [ "$(LAYERS)" -eq "0" ]; then \
		echo "ERROR: No layers found even after generation."; \
		exit 1; \
	fi
	@echo "Installing Hammerspoon keymap overlay..."
	@mkdir -p "$(HAMMERSPOON_DIR)"

	@echo "→ Copying keymap-overlay.lua"
	@cp keymap-overlay.lua "$(HAMMERSPOON_OVERLAY)"

	@echo "→ Copying keymap images (L*.png)"
	@cp $(BUILD_DIR)/L*.png "$(HAMMERSPOON_DIR)/"

	@echo "→ Copying key-to-layer.json"
	@cp $(KEY_TO_LAYER_JSON) "$(HAMMERSPOON_DIR)/key-to-layer.json"

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
	$(RSVG) -d $(DPI) -p $(DPI) -o $@ $<

$(BUILD_DIR)/L%.svg: $(KEYMAP_DRAWER_YAML) | $(BUILD_DIR)
	$(KEYMAP) draw "$(KEYMAP_DRAWER_YAML)" -j "$(KEYBOARD_JSON)" -l "$(LAYOUT_NAME)" -s "L$*" > "$@"

$(KEYMAP_DRAWER_YAML): $(QMK_KEYMAP_JSON) | $(BUILD_DIR)
	$(KEYMAP) parse -q $(QMK_KEYMAP_JSON) -c $(COLUMNS) > $@

.PHONY: _force_build
_force_build:

QMK_KEYMAP_JSON_DEPS := scripts/generate_qmk_keymap_postprocess.py $(KEYCODES_JSON) $(CUSTOM_KEYCODES_JSON) $(QMK_KEYMAP_C)
ifeq ($(VIAL),true)
QMK_KEYMAP_JSON_DEPS += _force_build
endif

$(QMK_KEYMAP_JSON): $(QMK_KEYMAP_JSON_DEPS) | $(BUILD_DIR)
ifeq ($(VIAL),true)
	@echo "Dumping QMK JSON from VIAL EEPROM..."
	$(VITALY) save -f $(VITALY_JSON)
	@[ -s "$(VITALY_JSON)" ] || (echo "ERROR: No VIAL dump found at $(VITALY_JSON)"; exit 1)
	$(UV) run scripts/generate_qmk_keymap_from_vitaly.py --vitaly-json $(VITALY_JSON) --keyboard-json "$(KEYBOARD_JSON)" --layout-name "$(LAYOUT_NAME)" > "$@.raw.tmp" || (rm -f "$@.raw.tmp" && exit 1)
else
	@echo "Compiling QMK JSON from source..."
	$(QMK) c2json -kb $(QMK_KEYBOARD) -km $(QMK_KEYMAP) > "$@.raw.tmp" || (rm -f "$@.raw.tmp" && exit 1)
endif
	$(UV) run scripts/generate_qmk_keymap_postprocess.py "$@.raw.tmp" --custom-keycodes-json $(CUSTOM_KEYCODES_JSON) > "$@.tmp" || (rm -f "$@.tmp" "$@.raw.tmp" && exit 1)
	rm -f "$@.raw.tmp"
	mv "$@.tmp" "$@"

$(KEYCODES_JSON): scripts/generate_keycodes.py | $(BUILD_DIR)
	$(UV) run scripts/generate_keycodes.py --qmk-dir "$(QMK_HOME)" > "$@.tmp" || (rm -f "$@.tmp" && exit 1)
	mv "$@.tmp" "$@"

$(CUSTOM_KEYCODES_JSON): $(QMK_KEYMAP_C) scripts/generate_custom_keycodes.py $(KEYCODES_JSON) | $(BUILD_DIR)
	$(UV) run scripts/generate_custom_keycodes.py "$(QMK_KEYMAP_C)" --keycodes-json "$(KEYCODES_JSON)" > "$@.tmp" || (rm -f "$@.tmp" && exit 1)
	mv "$@.tmp" "$@"

$(KEY_TO_LAYER_JSON): $(QMK_KEYMAP_C) scripts/generate_key_to_layer.py | $(BUILD_DIR)
	$(UV) run scripts/generate_key_to_layer.py --keymap-c "$(QMK_KEYMAP_C)" > "$@.tmp" || (rm -f "$@.tmp" && exit 1)
	mv "$@.tmp" "$@"

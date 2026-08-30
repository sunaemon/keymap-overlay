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

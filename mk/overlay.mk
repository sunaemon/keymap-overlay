.PHONY: run-overlay
run-overlay:
	$(KEYMAP_OVERLAY) $(if $(SIMULATE),--simulate "$(SIMULATE)")

.PHONY: build-overlay
build-overlay:
	$(CARGO) build --release -p $(OVERLAY_PACKAGE)
ifeq ($(OS_FAMILY),linux)
	$(MAKE) build-qt-renderer
endif

.PHONY: build-qt-renderer
build-qt-renderer:
ifeq ($(OS_FAMILY),linux)
	$(CMAKE) -S "$(QT_RENDERER_SOURCE)" -B "$(QT_RENDERER_BUILD_DIR)" \
		-DCMAKE_BUILD_TYPE=Release \
		-DCMAKE_RUNTIME_OUTPUT_DIRECTORY="$(abspath target/release)"
	$(CMAKE) --build "$(QT_RENDERER_BUILD_DIR)" --config Release
else
	$(error build-qt-renderer is only available on Linux)
endif

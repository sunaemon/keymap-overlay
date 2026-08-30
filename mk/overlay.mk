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

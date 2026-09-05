.PHONY: lint
lint:
	$(MISE_DEV) exec -- lefthook run lint

# Checks Cargo.lock against the RustSec advisory database. Ignored advisories
# and the reasons for them live in .cargo/audit.toml.
.PHONY: audit
audit:
	$(CARGO_AUDIT) audit

.PHONY: test
test:
	$(UV) run pytest

.PHONY: test-installer-sh
test-installer-sh:
	./installer/tests/test_install_sh.sh

.PHONY: test-installer-ps
test-installer-ps:
	powershell -NoProfile -Command 'Invoke-Pester -Path installer/tests/install.Tests.ps1 -CI'

.PHONY: check-contracts
check-contracts:
	@generated="$$(mktemp)"; schema="$$(mktemp)"; checked="$$(mktemp)"; \
	trap 'rm -f "$$generated" "$$schema" "$$checked"' EXIT; \
	$(DISPLAY_MODEL_SCHEMA_COMMAND) > "$$generated"; \
	tr -d '\r' < "$$generated" > "$$schema"; \
	tr -d '\r' < "$(DISPLAY_MODEL_SCHEMA)" > "$$checked"; \
	if ! cmp -s "$$checked" "$$schema"; then \
		diff -u "$$checked" "$$schema"; \
		exit 1; \
	fi
	$(CARGO) test --package keymap-overlay-generator --features contract-schema contract::

.PHONY: test-rust
test-rust: check-contracts
	$(CARGO) test --workspace --exclude keymap-overlay-winui

# Linux's release go/no-go gate. The installer test covers upgrade rollback;
# the two E2E halves meet at the typed D-Bus contract the project owns.
.PHONY: test-release-acceptance-linux
test-release-acceptance-linux: test-installer-sh test-hid-to-dbus-e2e-linux test-dbus-to-renderer-e2e-linux

# macOS's release go/no-go gate. HID behavior is exercised by the shared
# runtime and Linux UHID test; this covers the native AppKit presentation path.
.PHONY: test-release-acceptance-macos
test-release-acceptance-macos: test-installer-sh test-appkit-e2e-macos

# Windows's release go/no-go gate. The installer test covers upgrade rollback;
# the simulated E2E test covers the native Rust presentation path.
.PHONY: test-release-acceptance-windows
test-release-acceptance-windows: test-installer-ps test-windows-e2e

.PHONY: test-windows-e2e
test-windows-e2e: build-overlay
ifeq ($(OS_FAMILY),windows)
	powershell -NoProfile -ExecutionPolicy Bypass -File overlay/platforms/windows/tests/test_wpf_e2e.ps1
else
	$(error test-windows-e2e is only available on Windows)
endif

.PHONY: test-appkit-e2e-macos
test-appkit-e2e-macos: build-overlay
ifeq ($(OS_FAMILY),macos)
	./overlay/platforms/macos/tests/test_appkit_e2e.sh
else
	$(error test-appkit-e2e-macos is only available on macOS)
endif

.PHONY: test-hardware-session-macos
test-hardware-session-macos:
ifeq ($(OS_FAMILY),macos)
	./overlay/platforms/macos/tests/test_hardware_session.sh
else
	$(error test-hardware-session-macos is only available on macOS)
endif

.PHONY: test-hardware-session-linux
test-hardware-session-linux:
ifeq ($(OS_FAMILY),linux)
	./overlay/platforms/linux/tests/test_hardware_session.sh
else
	$(error test-hardware-session-linux is only available on Linux)
endif

.PHONY: test-hardware-physical-reports-macos
test-hardware-physical-reports-macos:
ifeq ($(OS_FAMILY),macos)
	./overlay/platforms/macos/tests/test_hardware_physical_reports.sh
else
	$(error test-hardware-physical-reports-macos is only available on macOS)
endif

.PHONY: test-hardware-physical-reports-linux
test-hardware-physical-reports-linux:
ifeq ($(OS_FAMILY),linux)
	./overlay/platforms/linux/tests/test_hardware_physical_reports.sh
else
	$(error test-hardware-physical-reports-linux is only available on Linux)
endif

.PHONY: build-hil-driver-macos
build-hil-driver-macos:
ifeq ($(OS_FAMILY),macos)
	$(CARGO) build --release -p keymap-overlay-hil
else
	$(error build-hil-driver-macos is only available on macOS)
endif

.PHONY: build-hil-macos
build-hil-macos: build-hil-driver-macos
ifeq ($(OS_FAMILY),macos)
	mkdir -p target/hil/KeymapOverlayHIL.app/Contents/MacOS
	cp overlay/platforms/macos/tests/hil_accessibility_info.plist \
		target/hil/KeymapOverlayHIL.app/Contents/Info.plist
	xcrun swiftc -o target/hil/KeymapOverlayHIL.app/Contents/MacOS/keymap-overlay-macos-hil-ui \
		overlay/platforms/macos/tests/hil_accessibility.swift
	codesign --force --deep --sign - target/hil/KeymapOverlayHIL.app
else
	$(error build-hil-macos is only available on macOS)
endif

.PHONY: test-hardware-lifecycle-macos
test-hardware-lifecycle-macos:
ifeq ($(OS_FAMILY),macos)
	./overlay/platforms/macos/tests/test_hardware_lifecycle.sh
else
	$(error test-hardware-lifecycle-macos is only available on macOS)
endif

.PHONY: prepare-hardware-login-macos
prepare-hardware-login-macos:
ifeq ($(OS_FAMILY),macos)
	./overlay/platforms/macos/tests/test_hardware_login.sh prepare
else
	$(error prepare-hardware-login-macos is only available on macOS)
endif

.PHONY: verify-hardware-login-macos
verify-hardware-login-macos:
ifeq ($(OS_FAMILY),macos)
	./overlay/platforms/macos/tests/test_hardware_login.sh verify
else
	$(error verify-hardware-login-macos is only available on macOS)
endif

.PHONY: test-hardware-firmware-macos
test-hardware-firmware-macos:
ifeq ($(OS_FAMILY),macos)
	./firmware/tools/test_hardware_macos.sh
else
	$(error test-hardware-firmware-macos is only available on macOS)
endif

.PHONY: test-hardware-release-macos
test-hardware-release-macos:
ifeq ($(OS_FAMILY),macos)
	$(MAKE) test-hardware-firmware-macos
	$(MAKE) test-hardware-physical-reports-macos
	$(MAKE) test-hardware-session-macos
	$(MAKE) test-hardware-lifecycle-macos
	$(MAKE) prepare-hardware-login-macos
	@echo "Sign out and sign in, then run 'make verify-hardware-login-macos'."
else
	$(error test-hardware-release-macos is only available on macOS)
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
	$(MAKE) run-hid-to-dbus-e2e-linux
else
	$(error test-hid-to-dbus-e2e-linux is only available on Linux)
endif

.PHONY: run-hid-to-dbus-e2e-linux
run-hid-to-dbus-e2e-linux:
ifeq ($(OS_FAMILY),linux)
	$(CC) -o target/virtual-raw-hid \
		-std=c11 -Wall -Wextra -Wpedantic -Werror \
		overlay/platforms/linux/tests/virtual_raw_hid.c -llzma
	dbus-run-session -- ./overlay/platforms/linux/tests/test_hid_to_dbus_e2e.sh
else
	$(error run-hid-to-dbus-e2e-linux is only available on Linux)
endif

# Runs both suites with coverage. The workspace-wide measurement remains
# informational; the contract core has the explicit threshold above.
.PHONY: test-contract-coverage
test-contract-coverage:
	$(CARGO_LLVM_COV) --package keymap-overlay-generator --lib --features contract-schema \
		--ignore-filename-regex '$(CONTRACT_COVERAGE_IGNORE)' --summary-only \
		--fail-under-lines 100

.PHONY: coverage
coverage: coverage-python coverage-rust test-contract-coverage

.PHONY: coverage-python
coverage-python:
	$(UV) run pytest --cov-branch --cov=model/scripts --cov=model/src --cov=installer/release --cov=firmware/tools --cov=tools.check_commit_message --cov-report=term-missing --cov-report=xml:coverage-python.xml

# Rust source is compiled differently on each host. CI uploads this report
# from Linux, macOS, and Windows so Codecov can merge the native backends.
.PHONY: coverage-rust
coverage-rust:
	$(CARGO_LLVM_COV) --workspace --exclude keymap-overlay-winui --all-targets --no-report
ifeq ($(OS_FAMILY),linux)
	@if [ -r /dev/uhid ] && [ -w /dev/uhid ]; then \
		$(MAKE) coverage-rust-hid-to-dbus-e2e-linux; \
	else \
		echo 'Skipping virtual HID coverage: /dev/uhid is not readable and writable'; \
	fi
endif
	$(CARGO_LLVM_COV) report --lcov --output-path coverage-rust.lcov
	$(CARGO_LLVM_COV) report --summary-only

.PHONY: coverage-rust-hid-to-dbus-e2e-linux
coverage-rust-hid-to-dbus-e2e-linux:
ifeq ($(OS_FAMILY),linux)
	@export CARGO_TARGET_DIR="$(abspath target/llvm-cov-target)"; \
		eval "$$($(CARGO_LLVM_COV) show-env --sh)"; \
		$(CARGO) build --package keymap-overlay-linux-daemon; \
		KEYMAP_OVERLAY_E2E_DAEMON="$(abspath target/llvm-cov-target/debug/keymap-overlay)" \
		$(MAKE) run-hid-to-dbus-e2e-linux
else
	$(error coverage-rust-hid-to-dbus-e2e-linux is only available on Linux)
endif

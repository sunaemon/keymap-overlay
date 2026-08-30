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

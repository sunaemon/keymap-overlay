# Release Checklist

Releases are beta until the project explicitly declares 1.0 stability.

1. Choose the next semantic version and set it in `[workspace.package]` in
   `Cargo.toml` and `[project]` in `pyproject.toml`.
2. Run `cargo check --workspace` to update and verify `Cargo.lock`.
3. Run `make licenses` and commit the updated
   `THIRD-PARTY-LICENSES.html` if dependencies changed.
4. Run the full verification suite:

   ```bash
   make format
   make lint
   make test
   make test-rust
   make test-installer-sh
   make build-overlay
   make audit
   ```

5. Verify the native overlay manually on every affected platform, including
   startup at login, upgrade, rollback after a simulated service failure, and
   uninstall. Verify firmware compile and flash when firmware changed.
6. Merge the release preparation and create an annotated `vMAJOR.MINOR.PATCH`
   tag on that exact commit.
7. Confirm the Release workflow publishes all three platform archives,
   `install.sh`, `install.ps`, `SHA256SUMS`, the MIT license and third-party
   notices, and GitHub artifact attestations.
8. Download each archive, verify its SHA-256 checksum and attestation, inspect
   its license files, then smoke-test the documented install and uninstall
   commands.
9. Review the generated release notes for beta status, compatibility changes,
   migrations, and known limitations before announcing the release.

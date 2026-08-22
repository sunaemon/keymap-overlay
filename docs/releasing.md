# Release Checklist

Releases are beta until the project explicitly declares 1.0 stability.

1. Choose the next semantic version and run the version bump script:

   ```bash
   make bump-version VERSION=MAJOR.MINOR.PATCH
   ```

   This updates `Cargo.toml` and `pyproject.toml`, runs
   `cargo check --workspace` and `uv lock` to refresh both lockfiles, and
   regenerates `THIRD-PARTY-LICENSES.html`.

2. Run the full verification suite:

   ```bash
   make format
   make lint
   make test
   make test-rust
   make test-installer-sh
   make build-overlay
   make audit
   ```

   On macOS, `make test-release-acceptance-macos` combines the installer and
   release build with the AppKit E2E test. On Linux,
   `make test-release-acceptance-linux` does the same with both Linux E2E
   halves.

   On Windows PowerShell, also run this before creating the release tag:

   ```powershell
   Invoke-Pester -Path installer/tests/install.Tests.ps1 -CI
   ```

3. Verify the native overlay manually on every affected platform, including
   startup at login, upgrade, rollback after a simulated service failure, and
   uninstall. Verify firmware compile and flash when firmware changed.
4. Open a release preparation PR containing the version bump, then merge it
   into `main`. Do not create the tag manually. After the `main` push CI passes,
   the Release workflow verifies that the tested commit came from a merged PR
   and changed the version, then creates both the `vMAJOR.MINOR.PATCH` tag and
   GitHub release on that exact commit.
5. Confirm the automated Release workflow publishes all four platform archives
   (Linux, macOS, Windows x64, and Windows ARM64),
   `install.sh`, `install.ps1`, `SHA256SUMS`, the MIT license and third-party
   notices, and GitHub artifact attestations.
6. Download each archive, verify its SHA-256 checksum and attestation, inspect
   its license files, then smoke-test the documented install and uninstall
   commands.
7. Review the generated release notes for beta status, compatibility changes,
   migrations, and known limitations before announcing the release.

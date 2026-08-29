# Release Checklist

Releases are beta until the project explicitly declares 1.0 stability.

1. Choose the next semantic version and run the version bump script:

   ```bash
   make bump-version VERSION=MAJOR.MINOR.PATCH
   ```

   This updates `Cargo.toml` and `pyproject.toml`, runs
   `cargo check --workspace` and `uv lock` to refresh both lockfiles, and
   regenerates the overlay third-party license notice.

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

3. Open a release preparation PR with
   [the release template](../.github/PULL_REQUEST_TEMPLATE/release.md) and wait
   for the automated build and test checks to pass. The `hardware-release-gate`
   check is expected to remain incomplete until step 4. Freeze the release
   candidate before beginning physical testing.
4. **Hardware release gate:** after CI passes and before merging, complete the
   [hardware pre-release checklist](#hardware-pre-release-checklist) already in
   the release preparation PR template and follow the exact
   [hardware test procedure](hardware-release-testing.md). Complete every
   required renderer and keyboard coverage row, record the tested PR head
   commit, and record the result of every item. Editing the PR reruns
   `hardware-release-gate`; it must pass before merge. Record upgrade, local
   simulated-service-failure rollback acceptance, and live uninstall as three
   explicit `PASS` results for each platform in the template's lifecycle table,
   with a terminal transcript or PR comment named in its evidence cell. Login
   startup is recorded by each platform's `*-10` check rather than duplicated
   in that table.
5. Merge the release preparation PR into `main` only after the hardware gate
   passes. Do not create the tag manually. After the `main` push CI passes, the
   Release workflow verifies that the tested commit came from a merged PR and
   changed the version, then creates both the `vMAJOR.MINOR.PATCH` tag and
   GitHub release on that exact commit.
6. Confirm the automated Release workflow publishes all five platform archives
   (Linux x64, Linux ARM64, macOS, Windows x64, and Windows ARM64),
   `install.sh`, `install.ps1`, `SHA256SUMS`, the MIT license and third-party
   notices, and GitHub artifact attestations.
7. Download each archive, verify its SHA-256 checksum and attestation, inspect
   its license files, then smoke-test the documented install and uninstall
   commands.
8. Review the generated release notes for beta status, compatibility changes,
   migrations, and known limitations before announcing the release.

## Hardware Pre-release Checklist

This is a pre-merge release gate, not a post-release smoke test. Fill a copy in
the release preparation PR after its automated checks pass and the candidate is
otherwise ready to merge; do not check off the reusable list in this file. Test
the PR head with the native overlay for the platform rather than `--simulate`.
The [release gate coverage map](release-test-coverage.md) explains why each
manual item still exists and what automation can eventually replace it.

Record the PR head commit, operating system, CPU architecture, desktop and
session, keyboard, `KEYBOARD_ID`, and firmware revision for each run. Every
shipped OS/architecture pair requires physical evidence; CI and simulation do
not substitute for an ARM64 run. Any behavior-affecting change after testing
invalidates the affected results: wait for CI on the new PR head and repeat
those hardware checks before merging. The required coverage matrix,
preparation commands, platform commands, and physical actions are in
[Hardware Release Test Procedure](hardware-release-testing.md).

The template deliberately separates three kinds of evidence:

1. `GLOBAL-01` and `GLOBAL-02` are release-wide firmware conditions. They may
   use a reasoned `N/A` only when the release delta changes no firmware or
   embedded overlay metadata; the gate derives this eligibility from the
   previous release tag and candidate commit.
2. Keyboard coverage records every bundled keyboard, one encoder keyboard, and
   all bundled keyboards connected simultaneously. The encoder row includes
   physical shaft/direction-wiring and push-switch observations. Exact-head HIL
   may supply the direction-label and mapped-output evidence; the simultaneous
   row includes model identity and most-recent-keyboard ownership.
3. Each required platform and architecture has its own ten checks. The stable
   prefixes are `MAC`, `KDE`, `GNOME`, `KDEA`, `WIN`, and `WINA`; a result from
   one prefix never satisfies another.

The ten platform checks have the same meaning on every backend. Their numeric
suffixes follow increasing human interaction:

| Case | Human operation                                                                    | Requirement and rationale                                                                     |
| ---- | ---------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------- |
| `01` | Run the startup/log target.                                                        | Native service startup and a real device-owned Vial model are outside CI.                     |
| `02` | None with exact-head HIL; otherwise hold two `MO` keys.                            | Proves report ordering, numeric precedence, restoration, and final hide.                      |
| `03` | None with HIL; otherwise make one reversible Vial edit and restart.                | Proves the intentional startup-only model reread.                                             |
| `04` | None with the macOS signed probe; otherwise type and click through the overlay.    | Real desktop focus, z-order, and pointer routing cannot be inferred from rendering tests.     |
| `05` | Compare the native overlay with live Vial.                                         | A person still judges clipping, native glyphs, geometry, transparency, and highlighted state. |
| `06` | Inspect every affected display and scale.                                          | Real compositor topology and DPI behavior differ from fixed-scale CI.                         |
| `07` | Type physically before and after; confirm USB and `KEYBOARD_ID`.                   | Proves ordinary matrix input and end-to-end device identity.                                  |
| `08` | Tap every physical `MO` switch once; use HIL for repeat scenarios where available. | Requested reports cannot prove the switch-to-firmware boundary.                               |
| `09` | Unplug/replug, or operate a switched USB port.                                     | Proves real OS removal, arrival, and startup-presence behavior.                               |
| `10` | Sign out, sign in, then press the first layer key.                                 | Authentication and graphical-session creation are deliberate session boundaries.              |

For macOS, the approved HIL procedure may compose `MAC-08` from a guided
physical switch-to-report transcript and deterministic report-to-AppKit
assertions through the real keyboard. `MAC-02`, `MAC-03`, and `MAC-04` may use
their exact-head HIL session assertions. The checklist results remain required;
this split reduces repeated human input and does not turn requested reports
into physical-switch evidence.

The person merging the release preparation PR owns the gate. It passes only
when every required coverage row is recorded, every applicable item passes,
every not-applicable global item has a reason, and the recorded PR head still
matches the candidate. CI validates the candidate SHA, both global checks, all
sixty platform checks, coverage, and lifecycle evidence before merge. The
Release workflow validates the same PR evidence again before publishing and
requires the tested PR head and published merge commit to have identical Git
trees. A missing or failed applicable result is a no-go. Exclude hardware or a
platform only by recording that scope decision in the PR and release notes.

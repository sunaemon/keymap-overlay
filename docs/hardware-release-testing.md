# Hardware Release Test Procedure

Use this procedure after the release PR's automated build and test jobs pass
and before merging it. Record every result in its matching release PR section. All
commands run from the candidate checkout unless a different shell is named.
See [Release Gate Coverage](release-test-coverage.md) for exactly what CI
already proves, what remains manual, and the E2E/HIL plan for retiring each
check.

The macOS HIL procedure deliberately splits switch-to-report firmware evidence
from report-to-overlay host evidence. This keeps the physical action small
without treating requested reports as physical switch evidence; see
[macOS Release-Gate Automation](macos-release-automation.md).

## Required Test Matrix

The proposed macOS, KDE Plasma, and Windows runs cover AppKit, Qt/LayerShellQt,
and WPF. They do not cover GNOME Shell, which is a separate Linux renderer. The
baseline release matrix is therefore:

| Platform ID                  | Architecture | Session and renderer                      | Required hardware coverage |
| ---------------------------- | ------------ | ----------------------------------------- | -------------------------- |
| `macos-arm64-appkit`         | arm64        | AppKit on macOS                           | One supported keyboard     |
| `linux-x86_64-kde-wayland`   | x86_64       | KDE Plasma on Wayland, Qt/LayerShellQt    | One supported keyboard     |
| `linux-x86_64-gnome-wayland` | x86_64       | GNOME 45 or newer on Wayland, GNOME Shell | One supported keyboard     |
| `linux-arm64-kde-wayland`    | arm64        | KDE Plasma on Wayland, Qt/LayerShellQt    | One supported keyboard     |
| `windows-x86_64-wpf`         | x86_64       | WPF on Windows 11                         | One supported keyboard     |
| `windows-arm64-wpf`          | arm64        | WPF on Windows 11                         | One supported keyboard     |

Each row has its own ten-item checklist; never use one row's result to complete
another row. Across those runs, test every bundled keyboard at least once, test
an encoder keyboard at least once, and test all keyboards together once. These
three release-wide requirements go in Keyboard coverage, not in every platform
checklist. This is not a full keyboard-by-platform Cartesian product. Every
shipped OS/architecture pair requires its own physical row and checklist;
never use CI, simulation, or an x86_64 run as ARM64 evidence. Renderer coverage
is orthogonal: KDE and GNOME are both required on Linux x86_64, while the
Linux ARM64 row uses KDE to exercise the native daemon, Qt renderer, and
LayerShellQt release binaries. Add KDE/X11 when Qt/X11 behavior changed. Record
any omitted renderer or session as an explicit release exclusion, but do not
exclude a shipped architecture.

## Runtime Model Preconditions

The runtime keeps models only in memory. Geometry, `KEYBOARD_ID`, encoder
placement, and sizing come from `keymapOverlay` metadata embedded in each
keyboard's Vial definition; key bindings come from live Vial EEPROM.

- When firmware or embedded overlay metadata changed, flash every affected test
  keyboard from the candidate.
- Connect every keyboard needed for the run before starting the overlay. A
  keyboard absent at startup has no model until the overlay is restarted.
- Do not use a generated JSON model to make the test pass. The runtime must
  operate with no `--asset-dir` or `--keyboard-config-dir` argument.
- Close the Vial application before starting the overlay so they do not contend
  for the same Raw HID interface.

## 1. Prepare the Candidate

Do not clear EEPROM or logs manually. Export any Vial configuration you want to
keep: `make flash` intentionally resets Vial EEPROM to compiled defaults. Keep
the old logs for comparison and note the test start time. `make install-overlay`
stops and replaces an existing overlay process safely.

In the candidate checkout:

```bash
git status --short
KMO_CANDIDATE_SHA="$(git rev-parse HEAD)"
printf 'Candidate: %s\n' "$KMO_CANDIDATE_SHA"
make clean
```

`git status --short` must be empty, and the printed SHA must exactly match the
release PR's head SHA. Do not begin hardware testing against a branch name or a
different commit. A later behavior-changing commit invalidates affected runs.

Record `GLOBAL-01` and `GLOBAL-02` once for the release. When firmware or
embedded overlay metadata changed, flash each affected
keyboard on macOS or Linux. For both bundled keyboards, put each board into its
bootloader when prompted:

```bash
make flash KEYBOARD_ID=1
make flash KEYBOARD_ID=2
```

After each flash, open Vial and verify that the compiled layout and bindings
appear. Make one harmless binding change, close Vial, unplug and reconnect the
keyboard, then reopen Vial and verify that the edit persisted. Restore the
binding, close Vial completely, and connect the keyboard before installing the
overlay.

## 2. Install and Inspect Each Platform

### macOS

Grant the installed binary Input Monitoring access in System Settings. Then:

```bash
KMO_TEST_STARTED_AT="$(date '+%Y-%m-%d %H:%M:%S')"
make install-overlay
launchctl print "gui/$(id -u)/com.sunaemon.keymap-overlay"
tail -n 100 ~/.local/var/log/keymap-overlay/overlay.log
grep -E -- '--asset-dir|--keyboard-config-dir' \
  ~/Library/LaunchAgents/com.sunaemon.keymap-overlay.plist
```

The launchd job must be running. The final `grep` must print nothing; an exit
status of 1 means the obsolete arguments are absent, which is the expected
result. The log must contain no HID-open or Vial-model error from this start.

### Linux with KDE Plasma

Run this subsection separately on x86_64 and ARM64. Confirm the desktop really
is KDE Wayland, install the Raw HID rule, then physically unplug and reconnect
the keyboard before installing:

```bash
printf 'Desktop: %s\nSession: %s\n' "$XDG_CURRENT_DESKTOP" "$XDG_SESSION_TYPE"
make install-udev-rules
KMO_TEST_STARTED_AT="$(date --iso-8601=seconds)"
make install-overlay
systemctl --user --no-pager --full status \
  keymap-overlay.service keymap-overlay-qt.service
journalctl --user -u keymap-overlay.service \
  --since "$KMO_TEST_STARTED_AT" --no-pager
systemctl --user cat keymap-overlay.service | \
  grep -E -- '--asset-dir|--keyboard-config-dir'
```

`XDG_CURRENT_DESKTOP` must identify KDE or Plasma and `XDG_SESSION_TYPE` must be
`wayland`. Both services must be active. The final `grep` must print nothing.
The journal must contain no permission, HID-open, Vial-read, or model error.

### Linux with GNOME

Log into a GNOME Wayland session with the same candidate checkout and keyboard:

```bash
printf 'Desktop: %s\nSession: %s\n' "$XDG_CURRENT_DESKTOP" "$XDG_SESSION_TYPE"
make install-udev-rules
make install-overlay
gnome-extensions enable keymap-overlay@sunaemon
systemctl --user restart keymap-overlay.service
systemctl --user --no-pager --full status keymap-overlay.service
gnome-extensions info keymap-overlay@sunaemon
```

The daemon must be active, the extension must be enabled, and the Qt renderer
must not draw a second overlay. Repeat the common physical checks below.

### Windows

Run this subsection separately on x86_64 and ARM64, using the
architecture-matching MSYS2 UCRT64 shell described in the README:

```bash
make install-overlay
```

Then run these checks in a non-administrator PowerShell:

```powershell
$KmoCandidateSha = git rev-parse HEAD
Get-Process keymap-overlay -ErrorAction Stop
$KmoRun = Get-ItemPropertyValue `
  -Path 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run' `
  -Name 'KeymapOverlay'
$KmoRun
Get-Content "$env:LOCALAPPDATA\keymap-overlay\logs\overlay.log" -Tail 100
```

The process must be running. The Run value must contain only the quoted path to
`keymap-overlay.exe`, with no `--asset-dir` or `--keyboard-config-dir`. The log
must contain no HID-open or Vial-model error from this start.

## 3. Run the Common Physical Checks

On macOS ARM64, run the two protocol-boundary proofs first:

```bash
make test-hardware-physical-reports-macos
make test-hardware-session-macos
```

The first target asks for one quick tap of every configured physical `MO` key
and verifies its expected ordered Raw HID press/release messages. The second
uses deterministic messages emitted by the already-flashed real keyboard to
exercise repeated show/hide, nested precedence, live Vial restart reads, and
interactive AppKit window safety. Their exact-head transcripts jointly satisfy
`MAC-03`; the second satisfies `MAC-04`, `MAC-07`, and `MAC-08`. No separate
physical two-key chord is required after every participating switch has passed
the first target. This exception applies only to macOS until another platform
has an equivalent release-binary HIL procedure.

Perform these checks on every row of the required platform matrix. Before and
after the run, verify normal keyboard input and matching USB/`KEYBOARD_ID`
identity for that platform's `*-01` check. The install-and-log inspection in
section 2 supplies `*-02`. The prefixes are `MAC`, `KDE`, `GNOME`, `KDEA`,
`WIN`, and `WINA`.

1. Except for the macOS split proof above, for `*-03`, with the keyboard connected before startup, hold every `MO` key. Verify the
   correct live Vial layer appears immediately, remains visible while held, and
   hides on release.
2. For `*-05`, compare geometry, platform-specific labels, custom glyphs, transparent keys,
   held-key highlighting, and encoder position with the live Vial layout.
3. Except for the macOS split proof above, also for `*-03`, tap each `MO` key quickly, then show and hide it at least ten times. Verify
   that no stale or stuck overlay remains.
4. Except for the macOS split proof above, for `*-04`, hold two `MO` keys. Verify numeric layer precedence, release the higher one
   and confirm the lower layer returns, then release the last key and confirm
   the overlay hides.
5. Except for the automated macOS AppKit assertions above, for `*-08`, in a text editor, continue typing while repeatedly showing the overlay. Click
   through the overlay as well. Every character and click must reach the editor;
   focus and the caret must not move. Explicitly verify the second and later
   show on Windows.
6. For `*-09`, on each affected monitor and scale factor, verify centering, size, topmost
   behavior, labels, and click-through behavior.
7. For the release-wide `encoder-keyboard` coverage row, hold the relevant layer
   on an encoder keyboard and rotate both directions, then push every encoder.
   Verify that each physical control is detected and that every push label,
   action, and position agrees. An exact-head encoder HIL transcript may replace
   manual verification of the counter-clockwise and clockwise labels and mapped
   host actions: it queues each direction through QMK's normal encoder path and
   captures the resulting live Vial-mapped USB event. It does not prove the
   shaft sensor, direction wiring, or push switch, so record those physical
   observations separately.
8. For `*-06`, while a layer is visible, unplug the keyboard. The overlay must hide. Plug
   the same keyboard back in without restarting; because its model was loaded
   at startup, layer events must resume.
9. Also for `*-06`, stop the overlay, unplug the keyboard, and start the overlay without it.
   Connecting it afterward must not display a model. Restart the overlay with
   the keyboard connected; physical layer events must then work. This is the
   intentional startup-read behavior.
10. Except for the automated macOS live-Vial session above, for `*-07`, stop the overlay, make a visible key-binding change in Vial, close Vial,
    and restart the overlay. The overlay must show the changed live binding.
    Restore the binding and restart once more.
11. For the release-wide `simultaneous-keyboards` coverage row, connect all
    supported keyboards before startup. Verify that each uses its
    own geometry and `KEYBOARD_ID`, and that the most recently used keyboard
    owns the overlay. Duplicate IDs must fail instead of selecting the wrong
    model.
12. For `*-10`, sign out and back in with the keyboard connected. Without manually starting
    anything, verify the service and renderer start and the first physical
    layer press works.

Stop the overlay before opening Vial or starting the absent-at-startup test,
then start it explicitly after closing Vial and reconnecting the keyboard:

```bash
# macOS: stop, then start
launchctl bootout "gui/$(id -u)/com.sunaemon.keymap-overlay"
launchctl bootstrap "gui/$(id -u)" \
  ~/Library/LaunchAgents/com.sunaemon.keymap-overlay.plist

# Linux KDE: stop, then start
systemctl --user stop keymap-overlay-qt.service keymap-overlay.service
systemctl --user start keymap-overlay.service keymap-overlay-qt.service

# Linux GNOME: stop, then start
systemctl --user stop keymap-overlay.service
systemctl --user start keymap-overlay.service
```

On Windows PowerShell, run the first command to stop and the last two to start:

```powershell
Get-Process keymap-overlay -ErrorAction SilentlyContinue | Stop-Process
$KmoExe = "$env:LOCALAPPDATA\Programs\keymap-overlay\keymap-overlay.exe"
Start-Process $KmoExe
```

## 4. Verify Installer Lifecycles

Run these operations locally on every platform row and keep the terminal
transcript or a PR comment containing the command results. The lifecycle table
requires a separate `PASS` for upgrade, rollback, and uninstall on each stable
platform ID. CI results are not lifecycle evidence for this table.

First verify a real upgrade from the previous release. Start from a clean
candidate worktree, install v0.0.7, then return to the exact candidate and
install it over the running previous version:

```bash
git switch --detach v0.0.7
make clean
make install-overlay
git switch release/0.0.8
git status --short
git rev-parse HEAD
make clean
make install-overlay
```

The status must be empty, the SHA must still match the release PR, the service
must be running, and the first physical layer press must work after the upgrade.
Use an equivalent detached checkout and candidate branch name for later
releases.

Next run the local simulated-service-failure rollback acceptance target. This
uses isolated temporary homes and asserts that an older binary, service state,
and user files are restored; it does not alter the live installation:

```bash
# macOS
make test-release-acceptance-macos

# Linux, on both KDE architecture rows and the GNOME x86_64 row
make test-release-acceptance-linux

# Windows, from each architecture-matching MSYS2 UCRT64 shell
make test-release-acceptance-windows
```

Finally exercise the live uninstall and verify that the process, login entry,
and installed binaries are absent. Logs are intentionally retained. Reinstall
the candidate afterward so the remaining login and hardware checks use the
same build:

```bash
# macOS
make uninstall-overlay
test ! -e ~/.local/bin/keymap-overlay
test ! -e ~/Library/LaunchAgents/com.sunaemon.keymap-overlay.plist
! launchctl print "gui/$(id -u)/com.sunaemon.keymap-overlay"
make install-overlay

# Linux
make uninstall-overlay
test ! -e ~/.local/bin/keymap-overlay
test ! -e ~/.config/systemd/user/keymap-overlay.service
test ! -e ~/.config/systemd/user/keymap-overlay-qt.service
! systemctl --user is-active --quiet keymap-overlay.service
! systemctl --user is-active --quiet keymap-overlay-qt.service
make install-overlay
```

On Windows, run `make uninstall-overlay` and `make install-overlay` in MSYS2
UCRT64. Between them, verify removal in a non-administrator PowerShell:

```powershell
if (Get-Process keymap-overlay -ErrorAction SilentlyContinue) { throw 'process remains' }
$RunPath = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
if (Get-ItemProperty -Path $RunPath -Name KeymapOverlay -ErrorAction SilentlyContinue) { throw 'Run entry remains' }
$KmoExe = "$env:LOCALAPPDATA\Programs\keymap-overlay\keymap-overlay.exe"
if (Test-Path $KmoExe) { throw 'binary remains' }
```

## 5. Record the Gate

Fill the release PR template without changing its stable row IDs or headers:

- The candidate line contains the exact 40-character PR head SHA.
- Every platform row contains the required architecture, OS version, desktop
  and session, keyboard name, `KEYBOARD_ID`, and firmware revision.
- The keyboard coverage rows name every bundled keyboard, at least one bundled
  encoder keyboard, and one simultaneous all-keyboard run. IDs must be decimal,
  comma-separated values.
- `GLOBAL-01` and `GLOBAL-02` are checked with `PASS`, or a reasoned `N/A` when
  firmware and embedded metadata did not change.
- Every `MAC`, `KDE`, `GNOME`, `KDEA`, `WIN`, and `WINA` check is completed
  independently with `PASS`; no architecture or platform result can be copied
  to another row.
- Every lifecycle row records `PASS` for upgrade, rollback, and uninstall and
  identifies its local transcript or PR comment in the evidence cell.

Do not mark a physical platform or coverage item from simulation or CI. The
`hardware-release-gate` check must pass before merge.

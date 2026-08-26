# macOS Release-Gate Automation

The macOS hardware-in-the-loop (HIL) runner combines a real, self-describing
Vial keyboard with deterministic firmware reports and macOS Accessibility
assertions. It runs the release overlay without `--simulate` and saves each
operation's transcript under `~/.local/var/log/keymap-overlay/hil/`.

This automation supplements the hardware release gate until it is required on
a dedicated macOS ARM64 runner and has demonstrated that it fails for the
regressions each checklist item describes. Do not mark an existing physical
item from these targets alone or weaken the gate to accept their output.

## Safety Boundary

The firmware accepts one versioned VIA custom command. It can emit only a KMO
layer press or release report. It cannot inject a keyboard key, alter QMK's
active layer, reset EEPROM, enter the bootloader, or detach USB. The normal
physical `MO` path remains unchanged.

The host driver also exposes the existing Vial get, set, and reset operations
needed to verify startup rereads and EEPROM persistence. Destructive reset and
flash operations appear only in the explicitly named firmware target.

## One-Time Host Setup

Build the stable, ad-hoc-signed Accessibility probe:

```bash
make build-hil-macos
```

Add `target/hil/keymap-overlay-macos-hil-ui` to System Settings > Privacy &
Security > Accessibility. Keep the overlay binary in Input Monitoring as the
normal installation procedure requires. Rebuild first and grant the resulting
stable binary, because macOS associates the permission with its signed code.

Flash both bundled keyboards with this candidate before running the session
targets. The fully automated firmware target requires two site-specific
executables:

- `KMO_HIL_BOOTLOADER_CONTROL`: invoked as `enter KEYBOARD_ID`; it physically
  puts that board into its real bootloader.
- `KMO_HIL_USB_CONTROL`: invoked as `off KEYBOARD_ID` or `on KEYBOARD_ID`; it
  controls only that keyboard's USB port.

An individually switched USB hub plus a board-specific bootloader actuator can
provide these interfaces. Do not implement either helper by asking the HIL
firmware to fake removal or bootloader entry; those are the hardware behaviors
the test must retain.

## Targets

After configuring the two hardware-control executables below, run the complete
current-session sequence with:

```bash
KMO_HIL_BOOTLOADER_CONTROL=/path/to/bootloader-control \
KMO_HIL_USB_CONTROL=/path/to/usb-control \
make test-hardware-release-macos
```

It runs firmware/EEPROM/disconnect, AppKit/Accessibility, and lifecycle checks
in order, then prepares the post-login continuation. Sign out and sign in, and
finish with `make verify-hardware-login-macos`. The session boundary is split
because macOS terminates the shell that initiated sign-out.

### Firmware, EEPROM, and disconnect behavior

```bash
KMO_HIL_BOOTLOADER_CONTROL=/path/to/bootloader-control \
KMO_HIL_USB_CONTROL=/path/to/usb-control \
make test-hardware-firmware-macos
```

For each ID in `KMO_HIL_KEYBOARD_IDS` (default `1 2`), this target performs two
real flashes. It first records the compiled default, makes a Vial edit, flashes
with a new EEPROM epoch, and confirms the default returns. It then edits the
binding again, power-cycles the actual USB port, and confirms persistence. The
same controlled port verifies visible disconnect, reconnect, absence at
startup, and recovery after a real overlay restart.

### Live AppKit session

```bash
make test-hardware-session-macos
```

The session target temporarily changes a transparent base-layer position to
F13, restarts the installed candidate, and restores the original value in an
exit trap. Its signed probe asserts:

- clean native startup and absence of obsolete model arguments;
- the F13 live-Vial label through the AppKit Accessibility hierarchy;
- ten show/hide cycles and two-layer numeric precedence;
- continued typing focus, click-through, and topmost ordering;
- centering on every currently attached display;
- ownership and restoration with both bundled keyboards connected.

Override `KMO_HIL_KEYBOARD_ID`, `KMO_HIL_SECONDARY_KEYBOARD_ID`,
`KMO_HIL_PRIMARY_LAYER`, or `KMO_HIL_SECONDARY_LAYER` when testing another
valid pair.

### Upgrade, rollback, and uninstall

```bash
make test-hardware-lifecycle-macos
```

This creates a temporary detached worktree for `v0.0.7`, installs it, upgrades
the running service to the clean candidate, and verifies the first HIL event.
It then runs local rollback acceptance, performs a live uninstall with absence
assertions, and reinstalls the candidate. The temporary worktree is removed
through `git worktree remove` even after failure.

### Actual sign-out and sign-in

Prepare a one-shot login continuation:

```bash
make prepare-hardware-login-macos
```

Sign out and sign in normally. The continuation waits for the keyboard and
LaunchAgent, then runs the first layer event and Accessibility checks. Verify
and remove the continuation afterward:

```bash
make verify-hardware-login-macos
```

Authentication at the macOS login window remains a human security boundary.
A dedicated runner can remove that interaction only when it is intentionally
configured for automatic login; the project must not change a developer
machine's login security settings.

## Coverage and Remaining Physical Assertions

| Gate        | Automated evidence                                              | Physical assertion retained                     |
| ----------- | --------------------------------------------------------------- | ----------------------------------------------- |
| `GLOBAL-01` | Compile, controlled bootloader entry, flash, and return         | Real keyboard and bootloader controller         |
| `GLOBAL-02` | Compiled reset plus Vial persistence across switched USB power  | Real EEPROM and switched USB port               |
| `MAC-01`    | USB identity plus typing/focus capture                          | A matrix switch still types normally            |
| `MAC-02`    | Installed native startup, device model read, plist, and logs    | Real Raw HID/Vial device                        |
| `MAC-03`    | Repeated deterministic firmware KMO reports                     | Physical switch-to-QMK report path              |
| `MAC-04`    | Nested KMO reports and numeric precedence                       | Physical two-switch chord                       |
| `MAC-05`    | Live Vial F13, AppKit labels, geometry, and held layer          | Encoder rotation/push and visual review         |
| `MAC-06`    | Controlled physical USB removal, return, and absent startup     | Switched physical port                          |
| `MAC-07`    | Vial EEPROM edit and real process restart                       | Real EEPROM                                     |
| `MAC-08`    | Accessibility focus/typing, pointer click-through, window order | Interactive macOS desktop                       |
| `MAC-09`    | Window bounds on every attached display and scale               | Every release-relevant display must be attached |
| `MAC-10`    | Post-login LaunchAgent continuation and first HIL event         | Actual sign-out/sign-in session boundary        |
| Lifecycle   | Real upgrade/uninstall/reinstall plus isolated rollback         | Running user launchd session                    |

The physical matrix and encoder gaps need actuators or instrumented switch and
encoder fixtures before their manual coverage rows can be retired. The
firmware KMO command deliberately does not pretend to cover them.

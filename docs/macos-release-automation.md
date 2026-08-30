# macOS Release-Gate Automation

The macOS hardware-in-the-loop (HIL) procedure separates release evidence at
the Raw HID protocol boundary:

1. A guided physical check proves that every real `MO` matrix switch makes QMK
   emit the expected `(keyboard_id, layer, pressed)` report.
2. The host driver asks the already-flashed real keyboard to emit chosen KMO
   reports, and the installed release overlay proves parsing, state reduction,
   model composition, and AppKit rendering deterministically.
3. On the encoder keyboard, the driver queues chosen rotations through QMK's
   normal encoder path and the signed probe captures the live Vial-mapped USB
   keyboard outputs.

The second half runs without `--simulate`; the versioned firmware command emits
the same KMO report that the physical path emits. Flash the candidate HIL
firmware once, not once per report. The deterministic half never counts as
matrix-switch evidence, while the physical half does not need to repeat every
renderer scenario. Together, their exact-head transcripts may satisfy the
corresponding macOS checklist item. Each operation is saved under
`~/.local/var/log/keymap-overlay/hil/`.

## Safety Boundary

The firmware accepts one versioned VIA custom command. It can emit a KMO layer
press or release report, or queue a configured encoder index and direction.
The encoder action resolves the current live Vial binding through QMK's normal
encoder queue; the request cannot directly name an arbitrary keycode. It
rejects an encoder action whose effective live binding is `QK_BOOT`, so it
cannot alter QMK's active layer, reset EEPROM, enter the bootloader, or detach
USB. The normal physical `MO` and encoder-driver paths remain unchanged.

The host driver also exposes the existing Vial get, set, and reset operations
needed to verify startup rereads and EEPROM persistence. Destructive reset and
flash operations appear only in the explicitly named firmware target.

## One-Time Host Setup

Build the stable, ad-hoc-signed Accessibility probe:

```bash
make build-hil-macos
```

Add `target/hil/KeymapOverlayHIL.app` to System Settings > Privacy & Security >
Accessibility. Keep the overlay binary in Input Monitoring as the normal
installation procedure requires. Rebuild first and grant the resulting stable
app, because macOS associates the permission with its signed code.

Flash both bundled keyboards with this candidate before running the session
targets. The HIL command can then emit any valid layer press or release, plus
either direction for any configured encoder, without another flash. The fully
automated firmware target requires two site-specific executables:

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

It runs firmware/EEPROM/disconnect, guided physical report capture,
AppKit/Accessibility, and lifecycle checks in order, then prepares the
post-login continuation. Sign out and sign in, and finish with
`make verify-hardware-login-macos`. The session boundary is split because macOS
terminates the shell that initiated sign-out.

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

### Physical switch-to-report proof

```bash
make test-hardware-physical-reports-macos
```

The target installs the exact clean candidate and monitors its structured layer
event log. This reuses the release overlay's existing Input Monitoring grant
instead of requiring a second grant for a test-only command-line binary. It
prompts for one quick physical tap of each bundled `MO` key and accepts only an
ordered press then release for the expected keyboard ID and layer. The target
does not request a report through the HIL command, so each captured event comes
from the prompted physical switch. The default sequence is:

- Insixty far-right key on the Z row: keyboard 1, layer 1;
- Insixty bottom-left key: keyboard 1, layer 2;
- DOIO bottom-left key: keyboard 2, layer 3.

Override the set with `KMO_HIL_PHYSICAL_REPORTS="ID:LAYER ..."` when the bundled
keymaps change. This target never invokes the deterministic HIL layer command.
One physical tap proves both the press and release firmware messages; repeated
show/hide and nested-state coverage belongs to the deterministic session below.

### Live AppKit session

```bash
make test-hardware-session-macos
```

The session target temporarily changes a transparent base-layer position to
F13 and the six DOIO base encoder directions to A through F, restarts the
installed candidate, and restores every original value in an exit trap. Its
signed probe asserts:

- clean native startup and absence of obsolete model arguments;
- the F13 live-Vial label through the AppKit Accessibility hierarchy;
- ten show/hide cycles and two-layer numeric precedence;
- continued typing focus, click-through, and topmost ordering;
- centering on every currently attached display;
- ownership and restoration with both bundled keyboards connected.
- all six encoder labels and the ordered USB key events produced when firmware
  queues counter-clockwise and clockwise rotation for each encoder.

Override `KMO_HIL_KEYBOARD_ID`, `KMO_HIL_SECONDARY_KEYBOARD_ID`,
`KMO_HIL_PRIMARY_LAYER`, `KMO_HIL_SECONDARY_LAYER`,
`KMO_HIL_ENCODER_KEYBOARD_ID`, or `KMO_HIL_ENCODER_LAYER` when testing another
valid pair. The current session fixture expects the bundled three-encoder DOIO.

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

Sign out and sign in normally. The continuation waits for the keyboards and
LaunchAgent, then records their readiness. Make the first physical layer press
after the desktop appears. Verify from the signed-in interactive terminal; the
verification runs the layer-event and Accessibility checks there because macOS
does not grant Accessibility automation to the background login LaunchAgent.
It also removes the continuation afterward:

```bash
make verify-hardware-login-macos
```

Authentication at the macOS login window remains a human security boundary.
A dedicated runner can remove that interaction only when it is intentionally
configured for automatic login; the project must not change a developer
machine's login security settings.

## Coverage and Remaining Physical Assertions

| Gate        | Exact-head HIL evidence                                                              | Additional physical assertion                   |
| ----------- | ------------------------------------------------------------------------------------ | ----------------------------------------------- |
| `GLOBAL-01` | Compile, controlled bootloader entry, flash, and return                              | Real keyboard and bootloader controller         |
| `GLOBAL-02` | Compiled reset plus Vial persistence across switched USB power                       | Real EEPROM and switched USB port               |
| `MAC-01`    | Installed native startup, device model read, plist, and logs                         | Real Raw HID/Vial device                        |
| `MAC-02`    | Deterministic nested KMO reports and numeric precedence                              | None; switch report proof is inherited from 08  |
| `MAC-03`    | Real Vial EEPROM edit and real process restart                                       | None after the session transcript passes        |
| `MAC-04`    | Accessibility focus/typing, pointer click-through, and window order                  | None on the signed-in interactive desktop       |
| `MAC-05`    | Live Vial labels, encoder direction labels/output, geometry, and held layer          | Encoder sensor/push and visual review           |
| `MAC-06`    | Window bounds on every attached display and scale                                    | Every release-relevant display must be attached |
| `MAC-07`    | USB identity plus typing/focus capture                                               | A matrix switch still types normally            |
| `MAC-08`    | Every physical `MO` press/release report plus deterministic fast and repeated cycles | None after both transcripts pass                |
| `MAC-09`    | Controlled physical USB removal, return, and absent startup                          | Switched physical port                          |
| `MAC-10`    | Post-login LaunchAgent continuation and first HIL event                              | Actual sign-out/sign-in session boundary        |
| Lifecycle   | Real upgrade/uninstall/reinstall plus isolated rollback                              | Running user launchd session                    |

The guided matrix taps, physical encoder sensor, and push switch still need a
person until actuators or instrumented fixtures perform them. Synthetic
rotation replaces manual checking of all six live mapped outputs; it does not
claim that a shaft moved or a push switch closed.

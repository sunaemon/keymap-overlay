## Release preparation

Version: `MAJOR.MINOR.PATCH`

Complete this after automated build and test checks pass. Replace the candidate
below with the full PR head SHA used for every physical run. A new commit makes
this gate stale and requires affected checks to be repeated. Follow the
[exact hardware test procedure](https://github.com/sunaemon/keymap-overlay/blob/main/docs/hardware-release-testing.md)
and consult the
[manual-versus-E2E coverage map](https://github.com/sunaemon/keymap-overlay/blob/main/docs/release-test-coverage.md).

## Hardware Release Gate

Candidate commit: `0000000000000000000000000000000000000000`

Use the stable IDs, section headings, and table headers exactly as written; CI
parses them. Replace every `Pending` cell and result. List multiple keyboards
and IDs with commas.

### Platform test matrix

| Platform ID                | Architecture | OS version | Desktop / session    | Keyboard(s) | `KEYBOARD_ID(s)` | Firmware revision(s) |
| -------------------------- | ------------ | ---------- | -------------------- | ----------- | ---------------- | -------------------- |
| macos-arm64-appkit         | arm64        | Pending    | AppKit / Aqua        | Pending     | Pending          | Pending              |
| linux-x86_64-kde-wayland   | x86_64       | Pending    | KDE Plasma / Wayland | Pending     | Pending          | Pending              |
| linux-x86_64-gnome-wayland | x86_64       | Pending    | GNOME / Wayland      | Pending     | Pending          | Pending              |
| linux-arm64-kde-wayland    | arm64        | Pending    | KDE Plasma / Wayland | Pending     | Pending          | Pending              |
| windows-x86_64-wpf         | x86_64       | Pending    | WPF / desktop        | Pending     | Pending          | Pending              |
| windows-arm64-wpf          | arm64        | Pending    | WPF / desktop        | Pending     | Pending          | Pending              |

### Human-operation order and rationale

Within every platform section, checks are listed from the least human
interaction to the most disruptive operation. The suffix meanings are shared
by every platform prefix; `*-07` begins before the run and is completed after
it.

| Case   | Human operation required                                                                      | Rationale                                                                                                  |
| ------ | --------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------- |
| `*-01` | Run the installed-startup/log target; no per-event input.                                     | CI cannot prove native login-service startup or a real device-owned Vial model.                            |
| `*-02` | None where exact-head HIL exists; otherwise hold two `MO` keys.                               | This isolates report ordering, layer precedence, restoration, and final hide.                              |
| `*-03` | None where HIL performs the reversible Vial edit; otherwise edit once and restart.            | The runtime intentionally rereads Vial only at process startup.                                            |
| `*-04` | None for the macOS signed probe; otherwise type and click through the visible overlay.        | Focus, window order, and pointer routing belong to the interactive desktop/window manager.                 |
| `*-05` | Visually compare the native overlay with the live Vial model.                                 | Semantic assertions cannot fully judge native rendering, clipping, glyphs, and physical encoder placement. |
| `*-06` | Exercise every affected attached display and scale, then inspect placement.                   | Real compositor topology and DPI behavior are not represented by fixed-scale CI rendering.                 |
| `*-07` | Type on the physical keyboard before and after the run; confirm USB and `KEYBOARD_ID`.        | This proves ordinary matrix input and end-to-end physical device identity.                                 |
| `*-08` | Tap every physical `MO` switch once; use HIL for repeat/precedence scenarios where available. | Requested reports cannot prove the switch-to-firmware notification boundary.                               |
| `*-09` | Unplug/replug the keyboard, or operate an independently switched USB port.                    | A real OS removal/arrival transition is required to test device lifetime behavior.                         |
| `*-10` | Sign out, sign in, then make the first physical layer press.                                  | Authentication and graphical-session creation are intentional human/session boundaries.                    |

`GLOBAL-01` and `GLOBAL-02` are conditional firmware prerequisites: enter each
affected board's real bootloader, then verify first-boot defaults and a Vial
edit across real USB power loss. They exist because compilation cannot prove
deployment/recovery or EEPROM ownership. `bundled-keyboards` is record-only;
it adds no gesture beyond proving every shipped keyboard somewhere in the
matrix. `encoder-keyboard` retains physical shaft/direction and push-switch
observations because synthetic queue input cannot prove them.
`simultaneous-keyboards` requires all bundled devices connected and operated
once because a single-device fixture cannot prove model ownership. Lifecycle
requires running the real upgrade/rollback/uninstall target and retaining its
transcript because stubbed installer tests do not exercise the user service.

### Platform-independent checks

These are release-wide conditions, not results from one operating system. Use
`PASS`, or `N/A: <specific reason>` only when the release delta changes no
firmware or embedded metadata.

- [ ] **GLOBAL-01** — Result: PENDING — When firmware or embedded overlay metadata changed, `make compile` and `make flash KEYBOARD_ID=<id>` complete on macOS or Linux, and every affected keyboard returns from the bootloader without manual recovery.
- [ ] **GLOBAL-02** — Result: PENDING — After a required firmware flash, compiled defaults appear on first boot and a subsequent Vial edit survives reconnect, confirming that an ordinary boot does not reset EEPROM.

### Keyboard coverage

Across the platform runs, record every bundled keyboard. The encoder row also
requires every encoder's counter-clockwise, clockwise, and push labels and
actions to match its position. An exact-head encoder HIL transcript may prove
the two direction labels and mapped host outputs, but it does not replace the
physical shaft/direction-wiring or push-switch observations. The simultaneous
row also requires each model to use the correct ID and the most recently used
keyboard to own the overlay.

| Coverage ID            | Keyboard(s) | `KEYBOARD_ID(s)` | Platform ID(s) | Result  |
| ---------------------- | ----------- | ---------------- | -------------- | ------- |
| bundled-keyboards      | Pending     | Pending          | Pending        | PENDING |
| encoder-keyboard       | Pending     | Pending          | Pending        | PENDING |
| simultaneous-keyboards | Pending     | Pending          | Pending        | PENDING |

For every platform check, change the box and `Result` together. Platform checks
accept only `PASS`; they cannot be completed from another platform's result.

### macos-arm64-appkit checks

- [ ] **MAC-01** — Result: PENDING — Native startup loads the in-memory model directly from the device, uses no host model/config arguments, and reports no device-open, Vial-read, model, or Raw HID errors.
- [ ] **MAC-02** — Result: PENDING — Deterministic reports through the real keyboard for two held layers follow numeric precedence, restore the lower layer, and hide after the final release.
- [ ] **MAC-03** — Result: PENDING — Restarting after a live Vial edit rereads the model, with no device read on the layer-key hot path.
- [ ] **MAC-04** — Result: PENDING — The overlay remains topmost and click-through without taking keyboard focus.
- [ ] **MAC-05** — Result: PENDING — Geometry, platform labels, custom glyphs, transparent keys, and the highlighted held key match the live Vial keymap.
- [ ] **MAC-06** — Result: PENDING — Size and position are correct on every affected display and scale factor.
- [ ] **MAC-07** — Result: PENDING — The keyboard types normally before and after the run, and its USB identity and `KEYBOARD_ID` match its configuration directory.
- [ ] **MAC-08** — Result: PENDING — Every physical `MO` switch emits its matching press/release report; deterministic reports through the real keyboard prove held visibility, fast taps, and ten repeated show/hide cycles without stale state.
- [ ] **MAC-09** — Result: PENDING — Unplugging while visible hides the overlay; reconnecting works when loaded at startup, while a keyboard absent at startup requires a restart.
- [ ] **MAC-10** — Result: PENDING — After sign-out and sign-in, the service reads the connected keyboard and handles the first physical layer press without a manual restart.

### linux-x86_64-kde-wayland checks

- [ ] **KDE-01** — Result: PENDING — The daemon and Qt renderer start, load the in-memory model directly from the device without host model/config arguments, and report no permission, HID, Vial, model, or D-Bus errors.
- [ ] **KDE-02** — Result: PENDING — Two held `MO` keys follow numeric precedence, restore the still-held lower layer, and hide after the final release.
- [ ] **KDE-03** — Result: PENDING — Restarting after a live Vial edit rereads the model, with no device read on the layer-key hot path.
- [ ] **KDE-04** — Result: PENDING — The overlay remains topmost and click-through without taking keyboard focus.
- [ ] **KDE-05** — Result: PENDING — Geometry, platform labels, custom glyphs, transparent keys, and the highlighted held key match the live Vial keymap in Qt/LayerShellQt.
- [ ] **KDE-06** — Result: PENDING — Size and position are correct on every affected display and scale factor, using the Wayland layer-surface role.
- [ ] **KDE-07** — Result: PENDING — The keyboard types normally before and after the run, and its USB identity and `KEYBOARD_ID` match its configuration directory.
- [ ] **KDE-08** — Result: PENDING — Every `MO` key shows its layer while held and hides it on release; fast taps and ten repeated holds leave no stuck or stale overlay.
- [ ] **KDE-09** — Result: PENDING — Unplugging while visible hides the overlay; reconnecting works when loaded at startup, while a keyboard absent at startup requires a restart.
- [ ] **KDE-10** — Result: PENDING — After sign-out and sign-in, both services read the connected keyboard and handle the first physical layer press without a manual restart.

### linux-x86_64-gnome-wayland checks

- [ ] **GNOME-01** — Result: PENDING — The daemon and Shell extension start, load the in-memory model directly from the device without host model/config arguments, show no second Qt overlay, and report no permission, HID, Vial, model, or D-Bus errors.
- [ ] **GNOME-02** — Result: PENDING — Two held `MO` keys follow numeric precedence, restore the still-held lower layer, and hide after the final release.
- [ ] **GNOME-03** — Result: PENDING — Restarting after a live Vial edit rereads the model, with no device read on the layer-key hot path.
- [ ] **GNOME-04** — Result: PENDING — The overlay remains topmost and click-through without taking keyboard focus.
- [ ] **GNOME-05** — Result: PENDING — Geometry, platform labels, custom glyphs, transparent keys, and the highlighted held key match the live Vial keymap in GNOME Shell.
- [ ] **GNOME-06** — Result: PENDING — Size and position are correct on every affected display and scale factor in GNOME Shell.
- [ ] **GNOME-07** — Result: PENDING — The keyboard types normally before and after the run, and its USB identity and `KEYBOARD_ID` match its configuration directory.
- [ ] **GNOME-08** — Result: PENDING — Every `MO` key shows its layer while held and hides it on release; fast taps and ten repeated holds leave no stuck or stale overlay.
- [ ] **GNOME-09** — Result: PENDING — Unplugging while visible hides the overlay; reconnecting works when loaded at startup, while a keyboard absent at startup requires a restart.
- [ ] **GNOME-10** — Result: PENDING — After sign-out and sign-in, the daemon and extension read the connected keyboard and handle the first physical layer press without a manual restart.

### linux-arm64-kde-wayland checks

- [ ] **KDEA-01** — Result: PENDING — The daemon and Qt renderer start, load the in-memory model directly from the device without host model/config arguments, and report no permission, HID, Vial, model, or D-Bus errors.
- [ ] **KDEA-02** — Result: PENDING — Two held `MO` keys follow numeric precedence, restore the still-held lower layer, and hide after the final release.
- [ ] **KDEA-03** — Result: PENDING — Restarting after a live Vial edit rereads the model, with no device read on the layer-key hot path.
- [ ] **KDEA-04** — Result: PENDING — The overlay remains topmost and click-through without taking keyboard focus.
- [ ] **KDEA-05** — Result: PENDING — Geometry, platform labels, custom glyphs, transparent keys, and the highlighted held key match the live Vial keymap in Qt/LayerShellQt.
- [ ] **KDEA-06** — Result: PENDING — Size and position are correct on every affected display and scale factor, using the Wayland layer-surface role.
- [ ] **KDEA-07** — Result: PENDING — The keyboard types normally before and after the run, and its USB identity and `KEYBOARD_ID` match its configuration directory.
- [ ] **KDEA-08** — Result: PENDING — Every `MO` key shows its layer while held and hides it on release; fast taps and ten repeated holds leave no stuck or stale overlay.
- [ ] **KDEA-09** — Result: PENDING — Unplugging while visible hides the overlay; reconnecting works when loaded at startup, while a keyboard absent at startup requires a restart.
- [ ] **KDEA-10** — Result: PENDING — After sign-out and sign-in, both services read the connected keyboard and handle the first physical layer press without a manual restart.

### windows-x86_64-wpf checks

- [ ] **WIN-01** — Result: PENDING — Native startup loads the in-memory model directly from the device, uses no host model/config arguments, and reports no device-open, Vial-read, model, or Raw HID errors.
- [ ] **WIN-02** — Result: PENDING — Two held `MO` keys follow numeric precedence, restore the still-held lower layer, and hide after the final release.
- [ ] **WIN-03** — Result: PENDING — Restarting after a live Vial edit rereads the model, with no device read on the layer-key hot path.
- [ ] **WIN-04** — Result: PENDING — The overlay remains topmost and click-through without taking keyboard focus, including the second and later show.
- [ ] **WIN-05** — Result: PENDING — Geometry, platform labels, custom glyphs, transparent keys, and the highlighted held key match the live Vial keymap in WPF.
- [ ] **WIN-06** — Result: PENDING — Size and position are correct on every affected display and scale factor in WPF.
- [ ] **WIN-07** — Result: PENDING — The keyboard types normally before and after the run, and its USB identity and `KEYBOARD_ID` match its configuration directory.
- [ ] **WIN-08** — Result: PENDING — Every `MO` key shows its layer while held and hides it on release; fast taps and ten repeated holds leave no stuck or stale overlay.
- [ ] **WIN-09** — Result: PENDING — Unplugging while visible hides the overlay; reconnecting works when loaded at startup, while a keyboard absent at startup requires a restart.
- [ ] **WIN-10** — Result: PENDING — After sign-out and sign-in, the Run entry starts the overlay, reads the connected keyboard, and handles the first physical layer press without a manual restart.

### windows-arm64-wpf checks

- [ ] **WINA-01** — Result: PENDING — Native startup loads the in-memory model directly from the device, uses no host model/config arguments, and reports no device-open, Vial-read, model, or Raw HID errors.
- [ ] **WINA-02** — Result: PENDING — Two held `MO` keys follow numeric precedence, restore the still-held lower layer, and hide after the final release.
- [ ] **WINA-03** — Result: PENDING — Restarting after a live Vial edit rereads the model, with no device read on the layer-key hot path.
- [ ] **WINA-04** — Result: PENDING — The overlay remains topmost and click-through without taking keyboard focus, including the second and later show.
- [ ] **WINA-05** — Result: PENDING — Geometry, platform labels, custom glyphs, transparent keys, and the highlighted held key match the live Vial keymap in WPF.
- [ ] **WINA-06** — Result: PENDING — Size and position are correct on every affected display and scale factor in WPF.
- [ ] **WINA-07** — Result: PENDING — The keyboard types normally before and after the run, and its USB identity and `KEYBOARD_ID` match its configuration directory.
- [ ] **WINA-08** — Result: PENDING — Every `MO` key shows its layer while held and hides it on release; fast taps and ten repeated holds leave no stuck or stale overlay.
- [ ] **WINA-09** — Result: PENDING — Unplugging while visible hides the overlay; reconnecting works when loaded at startup, while a keyboard absent at startup requires a restart.
- [ ] **WINA-10** — Result: PENDING — After sign-out and sign-in, the Run entry starts the overlay, reads the connected keyboard, and handles the first physical layer press without a manual restart.

### Lifecycle results

Run the documented upgrade, local rollback-acceptance, and uninstall procedure
on every platform. Each operation must be `PASS`; identify the saved local log,
terminal transcript, or PR comment in `Evidence`.

| Platform ID                | Upgrade | Rollback | Uninstall | Evidence |
| -------------------------- | ------- | -------- | --------- | -------- |
| macos-arm64-appkit         | PENDING | PENDING  | PENDING   | Pending  |
| linux-x86_64-kde-wayland   | PENDING | PENDING  | PENDING   | Pending  |
| linux-x86_64-gnome-wayland | PENDING | PENDING  | PENDING   | Pending  |
| linux-arm64-kde-wayland    | PENDING | PENDING  | PENDING   | Pending  |
| windows-x86_64-wpf         | PENDING | PENDING  | PENDING   | Pending  |
| windows-arm64-wpf          | PENDING | PENDING  | PENDING   | Pending  |

Release exclusions: None.

Notes:

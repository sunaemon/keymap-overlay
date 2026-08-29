# Release Gate Coverage

The hardware gate exists only for behavior that automated tests do not prove.
This document maps every release-wide and platform-specific manual check to
current CI coverage, its remaining gap, and the automation needed to retire it.
“Partial” never means the hardware check may be skipped. An exact-head local
HIL target may perform a checklist sequence and supply its transcript as the
required hardware evidence; that automates the gate rather than bypassing it.
Build jobs for every
release architecture prove compilation and packaging only; they do not replace
the physical macOS ARM64, Linux x86_64 and ARM64, or Windows x86_64 and ARM64
rows in the release gate.

## What Current Acceptance Tests Prove

- Rust unit tests cover Raw HID parsing, active-layer reduction, multi-keyboard
  precedence, disconnect state, transparency composition, model geometry,
  labels, custom keycodes, and encoder model construction.
- Linux HID-to-D-Bus E2E creates a kernel `/dev/uhid` Raw HID device and proves
  that one synthetic KMO press and release reaches public D-Bus state. It uses
  `KEYMAP_OVERLAY_E2E_MODEL`, so it does not read a Vial definition or EEPROM.
- Linux D-Bus-to-Qt E2E uses `--simulate`, proves show/hide/show state and
  held-key metadata, and compares one offscreen software-rendered image. It
  does not run under KDE, LayerShellQt, Wayland, X11, or a real display scale.
- macOS AppKit and Windows WPF E2E use `--simulate` and prove that native view
  state transitions visible → hidden → visible again. They bypass physical HID
  and Vial startup reads and do not prove focus, topmost, or click-through
  behavior on an interactive desktop.
- Installer tests use temporary homes and stubbed service commands to cover
  install, upgrade, rollback, uninstall, service files, Run-key arguments, and
  legacy-cache cleanup. They do not exercise launchd, systemd, GNOME Shell, a
  Windows login, or a real graphical session.
- The macOS HIL targets use both bundled physical keyboards and the installed
  release binary. A guided target captures every physical `MO` switch's KMO
  press/release pair; deterministic commands through the real keyboard then
  exercise repeated and nested AppKit scenarios, a real Vial EEPROM edit and
  restart, every live encoder direction binding and resulting USB key event,
  and interactive focus, pointer, and window-order assertions. These are local
  exact-head gate results, not CI or simulation. Synthetic encoder queue input
  does not prove the physical shaft sensor or push switch.

## Manual Gate Mapping

| Gate                                   | Current automation                                                                                             | What remains manual                                                                                | Automation needed to retire it                                                                              |
| -------------------------------------- | -------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------- |
| `GLOBAL-01` compile and flash          | Makefile and metadata-generation unit tests cover build inputs; release acceptance does not compile QMK        | QMK compilation, bootloader entry, deployment, return, and recovery                                | CI firmware compilation plus a flash rig with a USB relay and bootloader-volume/programmer control          |
| `GLOBAL-02` EEPROM ownership           | Metadata generation is tested; no automated test executes the firmware EEPROM hooks                            | First-boot reset and later Vial persistence on real EEPROM                                         | HIL power-cycle and Vial read/write assertions across flash and reboot                                      |
| Platform `*-01` typing and identity    | Compile-time ID validation and metadata tests                                                                  | Real USB identity and unchanged keyboard input on each host                                        | HIL keyboard plus host typing/input capture on every platform                                               |
| Platform `*-02` live startup model     | Metadata serialization and pure model building have unit tests                                                 | Native service/renderer startup, HID enumeration, embedded definition, and clean logs on each host | A virtual Vial Raw HID device that answers the complete startup protocol on every host                      |
| Platform `*-03` physical `MO` events   | Linux UHID proves one KMO pair; macOS HIL captures every physical pair and drives deterministic AppKit repeats | Equivalent switch report proof plus release-renderer scenarios remains on Linux and Windows        | Virtual Vial+KMO E2E on every OS, plus HIL firmware report verification                                     |
| Platform `*-04` held-layer precedence  | Unit tests cover ordering; macOS HIL drives nested reports through the real keyboard and AppKit                | End-to-end release-renderer scenarios remain on Linux and Windows                                  | Multi-report E2E scenarios using release binaries on all frontends                                          |
| Platform `*-05` geometry and labels    | Generator unit tests cover geometry, platform labels, custom labels, and transparency; Qt has one golden image | Candidate firmware metadata and live Vial state render correctly on every frontend                 | Virtual Vial fixtures plus golden/semantic assertions for AppKit, GNOME, Qt, and WPF                        |
| `encoder-keyboard` coverage            | Generator tests cover placement; macOS HIL checks all live direction labels and mapped USB outputs             | Physical shaft/direction wiring, pushes, push labels/actions, and rendering on other frontends     | Encoder-rich virtual Vial fixtures on every frontend plus instrumented rotation/push input                  |
| Platform `*-06` disconnect and arrival | Reducer disconnect and arrival-coalescing unit tests                                                           | OS arrival watchers, physical removal, and reconnect on macOS/Linux/Windows                        | Disconnectable virtual HID devices on each OS exercising the release process                                |
| Platform `*-07` Vial edit and restart  | Decoders have unit tests; macOS HIL edits real Vial EEPROM and asserts the changed model after process restart | The equivalent real-device restart sequence remains on Linux and Windows                           | Stateful virtual Vial device changed between real process starts; HIL persistence remains in `GLOBAL-02`    |
| `simultaneous-keyboards` coverage      | Reducer tests cover multiple IDs; duplicate-ID loading has no HID E2E                                          | Enumeration of simultaneous physical devices, duplicate-ID rejection, and correct model ownership  | Multiple virtual Vial devices, including duplicate-ID failure and recent-owner scenarios                    |
| Platform `*-08` window safety          | macOS HIL asserts focus, typing, pointer pass-through, and window order on an interactive AppKit desktop       | Equivalent interactive coverage remains for GNOME, KDE, and both Windows architectures             | Interactive desktop E2E with a text editor, pointer injection, active-window assertions, and screen capture |
| Platform `*-09` displays and scaling   | One fixed-scale Qt offscreen golden image                                                                      | Real compositor placement, DPI, multi-monitor, Wayland layer role, and X11 behavior                | Nested/virtual desktops with multiple displays and scale factors, plus geometry and screenshot assertions   |
| Platform `*-10` login startup          | Installer tests validate generated registration data with stubs                                                | Real launchd/systemd/Run startup, desktop readiness, and first physical event                      | Self-hosted or VM runners that perform a real sign-out/sign-in cycle and assert the running overlay         |

GNOME has no frontend E2E today. The extension is installed and its JavaScript
is formatted, but CI does not load it into GNOME Shell or assert a rendered
actor. KDE's current golden test is Qt offscreen rendering, not a Plasma or
LayerShellQt integration test.

## Automation Roadmap

Implement automation in this order to remove the largest amount of repetitive
manual work:

1. Extend the Linux `/dev/uhid` fixture into a stateful virtual Vial device. It
   should serve embedded `keymapOverlay` metadata, dynamic layers, custom
   keycodes, encoders, and KMO reports, and support removal/recreation. This can
   automate most of platform checks `*-02` through `*-07` and both specialized
   coverage rows for the shared pipeline.
2. Reuse deterministic encoder-, transparency-, nested-layer-, and
   multi-keyboard fixtures in AppKit, Qt, and WPF E2E. Add a nested GNOME Shell
   test for the extension. This can retire frontend-content portions of
   platform checks `*-04` and `*-05`, plus encoder and simultaneous-keyboard
   coverage.
3. Add interactive desktop runners that type into an editor, inject pointer
   input, inspect the active window, and capture multiple displays/scales. This
   is required before retiring platform checks `*-08` or `*-09`.
4. Add real-session login tests for launchd, systemd plus GNOME/KDE, and the
   Windows Run key. This is required before retiring platform check `*-10`.
5. Add a small HIL rack with both bundled keyboards and controlled USB power.
   Only HIL can fully retire `GLOBAL-01`, `GLOBAL-02`, platform check `*-01`,
   encoder sensor/push coverage, and the physical firmware portion of `*-03`.

An exact-head local HIL target may narrow the human actions inside a platform
row when it runs the installed release binary, preserves any irreducibly
physical input proof, fails on the regression it claims to cover, and records a
diagnostic transcript. The checklist item remains mandatory and cites that
transcript. Remove the item from the release gate entirely only when its
replacement runs on every affected backend as a required automated check.

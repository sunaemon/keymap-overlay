# Keymap Overlay Design

This project generates display assets for QMK keymap layers and displays the active
momentary layer in a native overlay, on macOS, Linux and Windows.

## Display Model Generation

Generation reads the connected device directly and needs no Python:

```text
VIAL EEPROM (live device, over Raw HID)
  ↓ keymap-overlay-generator (Rust; first-party Vial protocol client,
    one HID session)
  + self-describing keymapOverlay metadata embedded in the Vial definition
in-memory model map — every connected keyboard and layer
```

The normal runtime path writes no display model to disk. `generate_vial.py`
embeds `KEYBOARD_ID`, layout geometry, encoder placement, and sizing metadata
when firmware is built. At startup the native process combines that definition
with live Vial EEPROM state and retains the result only in memory.

## Runtime Data Flow

```text
Momentary layer key held
  ↓
QMK Raw HID report (KMO protocol)
  ↓
Rust HID listener (hidapi)
  ↓
native transparent window
  macOS: AppKit NSGlassEffectView (macOS 26+) or NSVisualEffectView + NSBox + NSTextField
  Linux daemon: final model over session D-Bus
    ├─ GNOME Shell extension (Wayland or X11)
    └─ Qt Quick + KDE LayerShellQt (other desktops)
  Windows: WPF
  ↓
active keyboard/layer models are composed and displayed
  ↓
matching layer key released
  ↓
previous held layer is restored, or the overlay is hidden when none remain
```

The overlay remains visible for the complete key hold. Within one keyboard,
held layers use QMK's numeric precedence and transparent keys fall through the
other active layers before the base layer. Between keyboards, the most recently
used keyboard owns the overlay. It hides once no momentary layers remain held.

### Startup Read

Before the listener starts (never on the keypress hot path above), the runtime
reads every connected self-describing Vial keyboard into memory. The Vial
client runs in the same process as the overlay: macOS and Windows ship no
second generator executable, while Linux keeps only its daemon and renderer
processes. A disconnected keyboard has no model in that process.

Vial does not send an external-change notification when its web application
writes EEPROM. A Vial edit therefore appears at the next startup read; the
user restarts the overlay after making a live edit. The runtime never polls or
writes the keymap itself.

## Raw HID Protocol

The firmware sends a 32-byte report over QMK's Raw HID interface when a
momentary layer key is pressed or released. The payload starts at byte zero
(or byte one when the operating system prepends a report ID):

| Bytes | Meaning                                        |
| ----- | ---------------------------------------------- |
| 0–2   | ASCII magic: `KMO`                             |
| 3     | Protocol version: `1`                          |
| 4     | Keyboard ID                                    |
| 5     | Layer number                                   |
| 6     | Pressed state (`1` for press, `0` for release) |
| 7–31  | Reserved and zero-filled                       |

The Rust listener selects the QMK Raw HID usage page (`0xFF60`) and usage
(`0x61`), then ignores every report that does not match this protocol. VIAL
uses the same HID interface, so unrelated VIAL traffic is ignored.

Bundled firmware also implements a narrow hardware-in-the-loop request through
VIA's keyboard-specific command `0xFC`:

| Bytes | Meaning                                                                                   |
| ----- | ----------------------------------------------------------------------------------------- |
| 0     | VIA keyboard command: `0xFC`                                                              |
| 1–4   | ASCII magic: `KMOH`                                                                       |
| 5     | HIL protocol version: `1`                                                                 |
| 6     | Action (`0` probe, `1` press report, `2` release report, `3` encoder CCW, `4` encoder CW) |
| 7     | Layer number for a report action, or zero-based encoder index for an encoder action       |
| 8     | Response status (`0` accepted, `1` invalid)                                               |
| 9–31  | Reserved and zero-filled                                                                  |

The firmware defers the resulting unsolicited KMO report or encoder queue
entry until VIA has sent the request response, so reports never overlap. Only
layers from `1` to `DYNAMIC_KEYMAP_LAYER_COUNT - 1` and configured encoder
indices are accepted. A report action emits the same overlay report as the
physical notification path. An encoder action enters QMK's normal bounded
encoder queue, so it resolves and emits the current live Vial encoder binding;
the request cannot directly choose an arbitrary keycode. Firmware rejects an
encoder action when that direction's effective live binding is `QK_BOOT`, so
the interface cannot change QMK's active layer, write EEPROM, detach USB, or
enter the bootloader. It therefore supplies deterministic frontend and
mapped-output coverage without being mistaken for matrix-switch,
encoder-sensor, or push-switch evidence.

Device arrival notifications request another enumeration without interrupting
healthy readers, so a release cannot be lost while the new device becomes
openable. Linux receives `hidraw` add notifications from udev, macOS receives
usage-filtered notifications from `IOHIDManager`, and Windows forwards
`WM_DEVICECHANGE` from the mapped WPF window. This restores HID event handling
for a keyboard whose model was loaded at startup. A keyboard absent at startup
requires an overlay restart before its model is available.

For hardware-free manual testing, `--simulate KEYBOARD_ID:LAYER` replaces the
HID listener with a synthetic source at the `LayerEventSink` boundary. It holds
the named layer for two seconds, releases it for one second, and repeats. This
exercises the real reducer, model composition, UI wakeup, and native renderer;
only HID enumeration, report parsing, and the physical firmware path are
bypassed.

## Native Overlay

`overlay/keymap-core` owns the Raw HID protocol and the pure active-layer state
reducer, including coalescing queued reports and device disconnections into the
final state change. `overlay/keymap-overlay-runtime` shares the keyboard
listener, maps core state changes to overlay transitions, and owns the asset
model, command-line handling, and logging.
Each platform owns its executable, device-arrival integration, and presentation
boundary. macOS owns an AppKit process, Windows owns a WPF process, and Linux
separates its HID daemon from replaceable renderer clients.

On **macOS** (`overlay/platforms/macos/appkit`) AppKit owns the
complete view hierarchy. The undecorated, always-on-top, click-through window
uses an `NSGlassEffectView` on macOS 26 and newer, with `NSVisualEffectView` as
the adaptive fallback on earlier releases. It composes the in-memory models as
native `NSBox` and `NSTextField` views inside that background.
There is no rasterized key or label foreground on macOS. Hiding swaps in an
empty content view and shrinks the still-mapped window to one pixel, avoiding
native show animations on every layer-key press.

The application replaces the former Hammerspoon and Lua integration entirely.
No synthetic function-key events or Hammerspoon configuration are required.

On **Windows**, `overlay/platforms/windows/wpf` owns the process and builds a native
WPF visual tree from each in-memory model. A narrow C ABI bridge loads the
models, shared Rust HID listener, and core state reducer. Rust invokes only a wake
callback; the WPF dispatcher calls back to take the final queued transition, so
bursts collapse before anything is drawn.

An experimental sibling executable in `overlay/platforms/windows/winui` exercises a
pure-Rust WinUI 3 frontend through Microsoft's experimental `windows-reactor`
crate. It calls the shared listener, core reducer, model loader, and composer
directly, so it has no C ABI bridge. `make build-winui-overlay` builds it on
Windows; normal builds, installation, and releases intentionally continue to
use WPF. The frontend uses Reactor's component-owned WinUI window and subclasses
its HWND to make it layered, non-activating, topmost, and click-through. Win32
owns only overlay window behavior; WinUI still owns controls, layout,
typography, DPI, and theme resources. Because WinUI 3 does not officially
support transparent top-level windows, the prototype must not replace WPF until
transparency and repeated-show focus behavior pass physical testing.

The transparent WPF window is mapped once and shrinks to one pixel while idle.
Its HWND uses `WS_EX_NOACTIVATE`, `WS_EX_TOOLWINDOW`, and click-through styling,
so repeated layer presses cannot take focus and the overlay stays out of the
taskbar and Alt-Tab. `SetWindowPos` always includes `SWP_NOACTIVATE`.

Windows publishes one self-contained `keymap-overlay.exe`. The .NET single-file
bundle contains the Rust bridge DLL and extracts native content automatically
before launch; installation and autostart still manage one executable.
Release builds publish matching WPF and Rust bridge binaries for both x64 and
ARM64 Windows; the installer selects the archive for the operating system's
native architecture.

On **Linux**, `overlay/platforms/linux/daemon` owns the startup-loaded models,
validates and composes them, owns
the reduced active-layer state, and publishes it on the user's D-Bus session.
`com.sunaemon.KeymapOverlay.Renderer1.GetState` returns a generation number,
visibility, and the final model JSON; `StateChanged` carries the same tuple.
Renderers subscribe before reading the snapshot, then discard older
generations. This closes the startup race without polling and keeps HID,
layer composition, and asset selection in one Rust process.

The GNOME Shell extension consumes that interface inside GNOME itself on both
Wayland and X11. It builds `St` actors from the semantic model, uses Shell
theme classes rather than fixed colours, and therefore follows the active
GNOME light or dark style. As shell-owned chrome it can be topmost,
click-through, and non-activating without asking Wayland for a privileged
foreign window role.

Other desktops start the `keymap-overlay-qt` client. It subscribes to the same
D-Bus interface through QtDBus, whose signal delivery wakes the Qt event loop
without polling. The C++ side builds keys, encoders, and labels as Qt Quick
items. The client exits cleanly under GNOME unless `KEYMAP_OVERLAY_FORCE_QT` is
set, preventing two renderers from drawing the same layer.

On Wayland, the QML window uses KDE LayerShellQt's attached `Window` API. It
requests the overlay layer, no keyboard interaction, no exclusive zone, and
placement on the active screen. On X11, the renderer omits the LayerShellQt
import and centres a conventional native overlay on the pointer's screen.
`Qt::WindowTransparentForInput` makes either surface click-through. No plain
Wayland application window can supply those semantics: the compositor must
grant the layer-surface role.

The Qt client is a standalone CMake application with no Rust/C++ boundary. The
Rust daemon retains HID access, protocol handling, transition state, and model
composition; Qt receives only the final semantic model over D-Bus and has no
access to HID or the asset directory.

The model uses platform-independent point geometry. On macOS those values are
AppKit points, on Linux they are Qt logical pixels, and on Windows they are WPF
device-independent units. `PIXELS_PER_UNIT` controls the size of
one QMK layout unit. WPF interprets them as device-independent units and applies
the active monitor's DPI scale when positioning the native window.

### Requirements on Linux

- GNOME Shell 45 or newer on Wayland or X11 uses the included shell extension.
  KDE Plasma and other supported Wayland desktops use Qt 6 Quick and the KDE
  LayerShellQt QML module. Qt is the preferred KDE renderer on both Wayland
  and X11, and also supports other non-GNOME desktops.
- Read access to the keyboard's `hidraw` node, which `make install-udev-rules`
  grants with a `uaccess` rule per keyboard. hidapi is built against hidraw
  rather than its default libusb backend: libusb would detach the kernel driver
  and stop the keyboard from typing, and only hidraw reports the usage page the
  Raw HID interface is selected by.

### Requirements on Windows

- Normal release installation through `install.ps1` requires only PowerShell.
  Make-based source development and builds additionally require an MSYS2
  UCRT64 shell with MSYS2's Git and GNU Make installed, and PowerShell
  reachable on `PATH` for writing the current user's Run key.
- Nothing has to be granted to read the keyboard: a vendor-defined HID
  interface is open to any process, unlike macOS Input Monitoring or the
  `hidraw` node on Linux. hidapi is built on its own Windows backend, which
  reports the usage page the Raw HID interface is selected by and needs no C
  compiler to build.

## Installation and Autostart

The platform installer downloads the latest versioned release archive,
requires a matching entry in `SHA256SUMS`, and, when the optional GitHub CLI is
present, verifies GitHub artifact attestations. The archive includes the native
overlay and notices; keyboard definitions and generated models are not release
artifacts.

Release archives carry the MIT license and generated third-party notices for
the overlay, which also serves anyone packaging it where a distribution
requires notices as files. The overlay binary embeds both notices and prints
them with `keymap-overlay --license` and `--third-party-licenses`, so a copy
carried away from its install directory still states its terms. The Windows
package also installs those files beside the executable, because WPF owns that
process and reaches the shared runtime through a C ABI that carries no strings,
leaving it nothing to print.

The installers stop the running service before replacing its files, preserve
the previous binaries, notices, and service definition
until the new service starts, and restore them if installation fails. Their
uninstall modes remove those installed files and the login entry while
retaining logs. Upgrades remove legacy cached model JSON.

Developers can instead use `make install-overlay`, which performs the following
source-build workflow:

`make install-overlay` performs the following steps:

1. Builds the platform executable. At each start it reads connected keyboards
   into memory before the Raw HID listener begins.
2. Installs it as
   `~/.local/bin/keymap-overlay` on macOS and Linux — with the Qt renderer
   beside it as `~/.local/bin/keymap-overlay-qt` — and as
   `%LOCALAPPDATA%/Programs/keymap-overlay/keymap-overlay.exe` on Windows.
   Where systemd or the login profile puts `~/.local/bin` on `PATH`,
   the Qt renderer can be diagnosed by running `keymap-overlay-qt`; otherwise
   `~/.local/bin/keymap-overlay-qt` names it directly. Either way the service
   definitions use absolute paths and do not depend on `PATH`.
3. Writes the per-user service definition:
   - macOS: the launchd agent
     `~/Library/LaunchAgents/com.sunaemon.keymap-overlay.plist`.
   - Linux: `keymap-overlay.service` for HID and D-Bus state plus
     `keymap-overlay-qt.service` for the non-GNOME renderer, both wanted by
     `graphical-session.target`. The GNOME extension is installed under the
     user's shell extension directory.
   - Windows: the current user's `KeymapOverlay` value under
     `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`.
4. Restarts the service so updates take effect immediately and starts it at
   future logins.

All service definitions start at login. macOS and Linux also restart after a
crash and stay stopped after a clean exit; Windows uses the per-user Run key,
which starts a fresh overlay at the next login.

One thing is specific to Windows: a running executable is locked there, so the
service is stopped before the binary is replaced rather than afterwards.

Where the log goes is an argument, `--log-out`, not an environment variable,
because the Run key carries arguments and no environment at all. Each system is
given the destination its supervisor handles best:

- Linux passes nothing, leaving the log on stderr for journald to timestamp,
  rotate and retain: `journalctl --user -u keymap-overlay`.
- macOS passes `~/.local/var/log/keymap-overlay/overlay.log`, because launchd
  redirects a job's output but never rotates it.
- Windows writes `%LOCALAPPDATA%\keymap-overlay\logs\overlay.log`. WPF owns
  that process and reaches the shared runtime through a C ABI that carries no
  strings, so it cannot be handed a path; `make install-overlay` refuses to run
  if `KEYMAP_OVERLAY_LOG_DIR` was overridden.

A log the overlay owns rotates at 1 MiB and retains the current file plus three
previous files.

`make uninstall-overlay` stops and removes the login service and installed
binary, and cleans up legacy cached models. It keeps logs for troubleshooting.

## Firmware Workflow

```text
firmware/examples/<keyboard>/keymap
  ↓ make flash KEYBOARD_ID=<keyboard>
QMK firmware with a fresh EEPROM epoch
  ↓
first boot: epoch differs → QMK initializes all EEPROM
  ↓
Vial initializes its dynamic keymap from the flashed keymap.c defaults
  ↓
keyboard device; later Vial web-app edits persist in EEPROM
```

The shared `firmware/layer_notify.h` helper is copied into the QMK keymap as
part of the firmware build. It constructs the `KMO` reports described above
and supplies the EEPROM-epoch comparison. Each keyboard calls it from
`keyboard_post_init_user`; on a new epoch it calls QMK's `eeconfig_init()`,
which clears all QMK and Vial EEPROM. Its `eeconfig_init_user` hook then saves
the new epoch so ordinary rebooting does not reset a Vial-edited keymap.

QMK source processing and firmware deployment do not run from the overlay's
Windows shell. QMK's toolchain there is QMK MSYS, separate from the MSYS2
UCRT64 shell that builds the overlay, so `compile` and `flash` point at WSL,
macOS, or Linux. Raw HID is not subject to that boundary: startup refresh reads
the device natively on every platform. This does not prevent manual flashing
of an already-built `.uf2`: Windows can mount the bootloader's `RPI-RP2` volume
and copy the file onto it in Explorer.

## Design Decisions

### VIAL over VIA

The project uses Vial because the connected keyboard is the source of truth for
the active display model. The first-party Vial client in
`keymap-overlay-generator` reads the dynamic keymap, encoder bindings, and the
keyboard's embedded Vial definition—including its custom-keycode identities—in
one Raw HID session. It does not write EEPROM.

`keymap.c` is the firmware configuration, not a second live keymap store.
`make flash` compiles it with a fresh EEPROM epoch. On the flashed firmware's
first boot, QMK clears EEPROM, and Vial initializes its dynamic keymap and
macros from those compiled defaults. After that, users edit the keymap in the
Vial web app; those EEPROM edits persist across restarts until the next
firmware flash.

The overlay reads that Vial state at startup, so a live edit is shown after an
explicit overlay restart. It does not poll the device or attempt to subscribe
to changes the Vial protocol cannot announce.

`generate_vial.py` embeds each custom keycode's name and display label from a
single whitespace-free comment token into the `vial.json` compiled into the
firmware (starting at Vial's fixed `QK_KB_0` keycode range). The overlay reads
that embedded metadata from the device after flashing, rather than consulting
the working copy of `keymap.c`.

### Shared Display Model

The in-process Vial model reader converts QMK's keymap and keyboard JSON into
one small, versioned display model per layer. The model contains only canvas geometry,
labels, transparency metadata, held-state metadata, and encoder actions; it
contains no toolkit-specific objects and does not pass through keymap-drawer,
YAML, SVG, or another schema. All three platforms retain these models only in
memory, compose held layers using QMK precedence, and render the result
with AppKit, GNOME Shell, Qt Quick, or WPF. Keys use quiet, nearly opaque fills
and a low-contrast
hairline so they stay distinct over bright and dark backgrounds; the held layer
key alone receives its pale tint. Display-only labels for custom keycodes come
from single whitespace-free comment tokens such as `α`, `USB-C`, or `PbyP` on
`custom_keycodes` entries in `keymap.c`. Generic and platform-specific aliases — arrow glyphs, ⌘/Super/⊞ for
the GUI key, and so on — are overlay-owned presentation policy, not keyboard
data: they live in `keymap-overlay-generator`'s built-in label tables keyed by
the current host platform. Encoder placement
is the only project-specific geometry: QMK knows the encoder count and pins
but not where knobs sit, so `config.json`
maps each encoder to its push-switch matrix position or to explicit `x`/`y`
layout coordinates. Matrix placement replaces the normal key drawing with one
circular knob, places counter-clockwise and clockwise actions above it, and
keeps its push action centred inside.

`keymap-overlay-generator` owns the canonical Rust semantic model types. The
runtime re-exports those same types for platform frontends instead of maintaining
a second representation or serializing models between the two linked libraries.

All runtimes read connected keyboards from Vial into memory at startup. Layer
events received between Vial responses are buffered and replayed through the
normal reducer when the live listener starts. Each accepted keyboard's open HID
handle and metadata keyboard ID transfer with its model into that listener, so
reports queued after the final Vial response stay in the same session and an
immediate disconnect can clear buffered state. Devices that do not produce a
self-describing model contribute neither a startup handle nor accepted layer
events. Events compose only accepted in-memory models. On Linux the daemon
sends the composed model to renderer clients, so no renderer leaves the
previous layer visible while disk I/O completes.

Events already waiting when a UI loop wakes are reduced to their final active
layer before the window changes. Intermediate restores and switches are not
drawn on the way to a newer layer or a hide, and the macOS window swaps to an
empty content view while hiding so a later map cannot expose stale content.

### Native Windows Rather Than One Toolkit

Each system gets the window integration it can actually support. AppKit covers
macOS, GNOME Shell renders inside GNOME, Qt Quick plus KDE LayerShellQt covers
other Linux Wayland sessions, and WPF covers Windows.
On Wayland a normal application window cannot raise itself above others or
reject input, which is the entire behaviour of this overlay; LayerShellQt is
therefore a semantic requirement rather than a styling choice.

Windows and macOS use different native-window implementations because macOS
can compose Liquid Glass and native controls directly while Windows needs the mapped,
non-activating behaviour described above.

Windows CI runs the release WPF executable with `--simulate`, then asserts the
visible, hidden, and visible-again native presentation states. This covers the
dispatcher wake-up, model composition, and WPF visual-tree path without a
physical keyboard. Focus, topmost, and click-through behavior still require a
real interactive desktop.

The cost is three windows to maintain, each exercised only by the CI job for
its own system. Linux CI also runs the release daemon and Qt renderer on an
isolated D-Bus session with the in-memory simulation fixture, then
asserts the visible, hidden, and visible-again states through the public D-Bus
contract and compares the software-rendered Qt Quick output with a golden PNG.
Another Linux CI test creates a vendor-defined device through `/dev/uhid` and
sends the real 32-byte Raw HID protocol through the kernel's `hidraw` path,
then asserts the visible and hidden states from the daemon's public D-Bus
contract. Together the two tests cover both sides of the daemon independently.
That a window stays on top, passes clicks through and never takes
focus still needs a real machine.

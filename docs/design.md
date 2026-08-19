# Keymap Overlay Design

This project generates display assets for QMK keymap layers and displays the active
momentary layer in a native overlay, on macOS, Linux and Windows.

## Display Asset Generation

Under the default `VIAL=true`, generation reads the connected device
directly and needs no Python:

```text
VIAL EEPROM (live device, over Raw HID)
  ↓ keymap-overlay-generator (Rust; vitaly as a library, one HID session)
  + keyboard.json + config.json
build/<keyboard>/assets/<platform>/<keyboard>.json — every layer, one file
  ↓ make install-assets
installed models directory/<keyboard>.json
```

`VIAL=false` (render straight from `keymap.c`, no device connected) keeps the
original Python pipeline, one process per layer, then consolidated:

```text
keymap.c
  ↓ QMK c2json
build/<keyboard>/qmk-keymap.json
  + keyboard.json + config.json + encoder map
  ↓ generate_overlay_asset.py, one process per layer
  ├─ macOS: build/<keyboard>/assets/macos/<keyboard>_L<n>.json
  ├─ Linux: build/<keyboard>/assets/linux/<keyboard>_L<n>.json
  └─ Windows: build/<keyboard>/assets/windows/<keyboard>_L<n>.json
  ↓ consolidate_layer_models.py
build/<keyboard>/assets/<platform>/<keyboard>.json
  ↓ make install-assets
installed models directory/<keyboard>.json
```

Either way, only the combined `<keyboard>.json` — every layer keyed by
number, in one file — is installed; any per-layer file on the `VIAL=false`
path is a build-time intermediate. `make install-assets` is the
platform-independent model-generation and copy target on both paths. It
installs JSON on every platform. On Windows, generate models from WSL with
`make install-assets`, then run native `make install-overlay`; WSL writes
them directly to `%LOCALAPPDATA%/keymap-overlay/`. On macOS and Linux the
installed models directory is `~/.cache/keymap-overlay`: a regenerable cache
of what the connected device already knows, not configuration.

## Runtime Data Flow

```text
Momentary layer key held
  ↓
QMK Raw HID report (KMO protocol)
  ↓
Rust HID listener (hidapi)
  ↓
native transparent window
  macOS: AppKit NSGlassEffectView + NSBox + NSTextField
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

### Startup Self-Heal

Before the listener starts (never on the keypress hot path above), the
runtime optionally fills in any keyboard missing from `--asset-dir`: given
`--keyboard-config-dir` (the Makefile passes `KEYBOARDS_DIR`), it shells out
to `keymap-overlay-generator` — installed alongside the frontend, not linked
in, for the same reason `keymap-overlay-generator` is its own Cargo
workspace — once per keyboard subdirectory whose `<id>.json` doesn't exist
yet. A keyboard that isn't currently connected is skipped with a log warning,
not a startup failure; it's picked up on the next restart once it is. This
only covers the Makefile-driven install (`install-overlay`); the generic
`install.sh`/`install.ps1` release installer has no equivalent
`--keyboard-config-dir` to point at, since it's a private, per-user path.

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

Device arrival notifications request another enumeration without interrupting
healthy readers, so a release cannot be lost while the new device becomes
openable. Linux receives `hidraw` add notifications from udev, macOS receives
usage-filtered notifications from `IOHIDManager`, and Windows forwards
`WM_DEVICECHANGE` from the mapped WPF window. This makes a reconnected keyboard
available even while another keyboard remains active.

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
complete view hierarchy. The
undecorated, always-on-top, click-through window uses an `NSGlassEffectView` as
its content. It parses the installed JSON at startup and caches composed layers
as native `NSBox` and `NSTextField` views inside the glass view's `contentView`.
There is no rasterized key or label foreground on macOS. Hiding swaps in an
empty content view and shrinks the still-mapped window to one pixel, avoiding
native show animations on every layer-key press.

The application replaces the former Hammerspoon and Lua integration entirely.
No synthetic function-key events or Hammerspoon configuration are required.

On **Windows**, `overlay/platforms/windows/wpf` owns the process and builds a native
WPF visual tree from each installed JSON model. A narrow C ABI bridge loads the
shared Rust HID listener and core state reducer. Rust invokes only a wake
callback; the WPF dispatcher calls back to take the final queued transition, so
bursts collapse before anything is drawn.

An experimental sibling executable in `overlay/platforms/windows/winui` exercises a
pure-Rust WinUI 3 frontend through Microsoft's unreleased `windows-reactor`
crate. It calls the shared listener, core reducer, model loader, and composer
directly, so it has no C ABI bridge. `make build-winui-overlay` builds it on
Windows; normal builds, installation, and releases intentionally continue to
use WPF. Because WinUI 3 does not officially support transparent top-level
windows, the prototype hosts its WinUI visual tree in a
`DesktopWindowXamlSource` attached to a layered Win32 popup. Win32 owns only
overlay window behavior; WinUI still owns controls, layout, typography, DPI,
and theme resources. The prototype must not replace WPF until XAML Island
transparency and repeated-show focus behavior pass physical testing.

The transparent WPF window is mapped once and shrinks to one pixel while idle.
Its HWND uses `WS_EX_NOACTIVATE`, `WS_EX_TOOLWINDOW`, and click-through styling,
so repeated layer presses cannot take focus and the overlay stays out of the
taskbar and Alt-Tab. `SetWindowPos` always includes `SWP_NOACTIVATE`.

Windows publishes one self-contained `keymap-overlay.exe`. The .NET single-file
bundle contains the Rust bridge DLL and extracts native content automatically
before launch; installation and autostart still manage one executable.

On **Linux**, `overlay/platforms/linux/daemon` loads and
validates the installed models, owns
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

The normal installation path separates generated assets from the native
application. `make install-assets` builds keyboard-specific JSON models
from the source checkout. The platform installer then downloads the latest versioned
release archive, requires a matching entry in `SHA256SUMS`, and, when the
optional GitHub CLI is present, verifies GitHub artifact attestations.

Release archives carry the MIT license and the generated third-party notices,
which also serves anyone packaging the overlay where a distribution requires
them as files. On macOS and Linux nothing installs them: the binary embeds both
and prints them with `keymap-overlay --license` and `--third-party-licenses`,
so a copy carried away from its install directory still states its terms. The
Windows package installs both beside the executable, because WPF owns that
process and reaches the shared runtime through a C ABI that carries no strings,
leaving it nothing to print.

The installers stop the running service before replacing its binary, preserve
the previous binary, notices and service definition until the new service
starts, and restore them if installation fails. Their uninstall modes remove
the executable, notices and login entry while retaining generated assets and logs.

Developers can instead use `make install-overlay`, which performs the following
source-build workflow:

`make install-overlay` performs the following steps:

1. On macOS and Linux, uses the `install-assets` target to generate and
   install all layer assets as JSON. On Windows, verifies
   that WSL has already generated JSON models under
   `%LOCALAPPDATA%/keymap-overlay/`.
2. Builds the platform executable and installs it as
   `~/.local/bin/keymap-overlay` on macOS and Linux — with the Qt renderer
   beside it as `~/.local/bin/keymap-overlay-qt` — and as
   `%LOCALAPPDATA%/Programs/keymap-overlay/keymap-overlay.exe` on Windows.
   Executables are kept apart from the generated models each system stores
   elsewhere. Where systemd or the login profile puts `~/.local/bin` on `PATH`,
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

`make uninstall-overlay` stops and removes the login service, installed binary,
and generated JSON models. It keeps the logs for troubleshooting.

## Firmware Workflow

```text
firmware/examples/<keyboard>/keymap
  ↓ make flash KEYBOARD_ID=<keyboard>
QMK firmware with RAW_ENABLE = yes
  ↓
keyboard device
```

The shared `firmware/layer_notify.h` helper is copied into the QMK keymap as
part of the firmware build. It constructs the `KMO` reports described above.

This workflow does not run from the overlay's Windows shell. QMK's toolchain
there is QMK MSYS, separate from the MSYS2 UCRT64 shell that builds the overlay,
so `compile`, `flash`, and `flash-keymap` stop with a message pointing at QMK
MSYS, WSL, macOS, or Linux. This does not prevent manual flashing of an
already-built `.uf2`: Windows can mount the bootloader's `RPI-RP2` volume and
copy the file onto it in Explorer. The images and the overlay itself are
unaffected, so a Windows user builds firmware once elsewhere and does
everything else natively.

## Design Decisions

### VIAL over VIA

The project uses VIAL because `vitaly` can read and write VIAL keymap data for
the EEPROM-based workflow, and because the connected keyboard is the default
source of truth for both halves of the display model. `keymap-overlay-generator`
(`overlay/keymap-overlay-generator`) depends on `vitaly` as a Rust library,
not just its CLI: `vitaly::protocol::load_layers_keys` reads the dynamic
keymap out of EEPROM and `vitaly::protocol::load_vial_meta` reads the
keyboard's own embedded Vial definition — including the identity of its
custom keycodes — directly from the device, in the same HID session.
`keymap.c` is not consulted for either, so a live edit made in the Vial app is
reflected without recompiling. `VIAL=false` renders straight from `keymap.c`
instead, with no device connected and no Rust involved — useful for keyboards
whose keymap is never edited outside source; that path stays the Python
pipeline described above.

`keymap.c` remains the source that firmware is compiled and flashed from either
way, and `generate_vial.py` embeds each custom keycode's name and
single-character display glyph into the `vial.json` compiled into that firmware
(starting at Vial's fixed `QK_KB_0` keycode range). Once a keyboard has been
flashed, VIAL-mode rendering reads its keymap and custom-keycode metadata from
the device rather than from the contents of `keymap.c`; the source file must
still remain present as a Make dependency.

### Shared Display Model

The generator — native Rust under `VIAL=true`, Python under `VIAL=false` —
converts QMK's keymap and keyboard JSON into one small, versioned display
model per layer. The model contains only canvas geometry,
labels, transparency metadata, held-state metadata, and encoder actions; it
contains no toolkit-specific objects and does not pass through keymap-drawer,
YAML, SVG, or another schema. All three platforms install these models as JSON,
compose the held layers in memory using QMK precedence, and render the result
with AppKit, GNOME Shell, Qt Quick, or WPF. Keys use quiet, nearly opaque fills
and a low-contrast
hairline so they stay distinct over bright and dark backgrounds; the held layer
key alone receives its pale tint. Display-only Unicode labels for custom
keycodes come from single-character comments on `custom_keycodes` entries in
`keymap.c`. Generic and platform-specific aliases — arrow glyphs, ⌘/Super/⊞ for
the GUI key, and so on — are overlay-owned presentation policy, not keyboard
data: they live in built-in label tables keyed by `OVERLAY_PLATFORM` (which
defaults to the current host) — `generate_overlay_asset.py`'s under
`VIAL=false`, `keymap-overlay-generator`'s `labels.rs` under `VIAL=true`, kept
in sync by hand. Encoder placement
is the only project-specific geometry: QMK knows the encoder count and pins
but not where knobs sit, so `config.json`
maps each encoder to its push-switch matrix position or to explicit `x`/`y`
layout coordinates. Matrix placement replaces the normal key drawing with one
circular knob, places counter-clockwise and clockwise actions above it, and
keeps its push action centred inside.

All runtimes parse every installed JSON model at startup. Layer events compose
only those in-memory models. On Linux the daemon sends the composed model to
renderer clients, so no renderer leaves the previous layer visible while disk
I/O completes.

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

The cost is three windows to maintain, each exercised only by the CI job for
its own system. What CI can prove is that each compiles and that the shared
logic passes; that a window stays on top, passes clicks through and never takes
focus still needs a real machine.

# Keymap Overlay Design

This project generates display assets for QMK keymap layers and displays the active
momentary layer in a native overlay, on macOS, Linux and Windows.

## Display Asset Generation

```text
keymap.c (or VIAL EEPROM with VIAL=true)
  ↓ QMK c2json / Vitaly export
build/<keyboard>/qmk-keymap.json
  + keyboard.json + config.json + encoder map
  ↓ first-party display-model generator
platform-neutral geometry, labels, transparency, and state (one model per layer)
  ├─ macOS: build/<keyboard>/assets/macos/<keyboard>_L<n>.json
  ├─ Linux: build/<keyboard>/assets/linux/<keyboard>_L<n>.json
  └─ Windows: build/<keyboard>/assets/windows/<keyboard>_L<n>.json
  ↓ make install-assets
platform configuration directory/<keyboard>_L<n>.json
```

`make install-assets` is the platform-independent model-generation and copy
target. It installs JSON on every platform. On Windows, generate models from
WSL with `make install-assets`, then run native `make install-overlay`; WSL
writes them directly to `%USERPROFILE%/.config/keymap-overlay/`.

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
active <keyboard>_L<layer> assets are composed and displayed
  ↓
matching layer key released
  ↓
previous held layer is restored, or the overlay is hidden when none remain
```

The overlay remains visible for the complete key hold. Within one keyboard,
held layers use QMK's numeric precedence and transparent keys fall through the
other active layers before the base layer. Between keyboards, the most recently
used keyboard owns the overlay. It hides once no momentary layers remain held.

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

`crates/keymap-overlay` shares the keyboard listener, transition reducer,
asset model, and logging. macOS owns its window behind `src/ui/`; Windows owns
its process in WPF; Linux separates the HID daemon from replaceable renderer
clients.

On **macOS** (`src/ui/appkit.rs`) AppKit owns the complete view hierarchy. The
undecorated, always-on-top, click-through window uses an `NSGlassEffectView` as
its content. It parses the installed JSON at startup and caches composed layers
as native `NSBox` and `NSTextField` views inside the glass view's `contentView`.
There is no rasterized key or label foreground on macOS. Hiding swaps in an
empty content view and shrinks the still-mapped window to one pixel, avoiding
native show animations on every layer-key press.

The application replaces the former Hammerspoon and Lua integration entirely.
No synthetic function-key events or Hammerspoon configuration are required.

On **Windows**, `windows/KeymapOverlay.Wpf` owns the process and builds a native
WPF visual tree from each installed JSON model. A narrow C ABI bridge loads the
shared Rust HID listener and transition reducer. Rust invokes only a wake
callback; the WPF dispatcher calls back to take the final queued transition, so
bursts collapse before anything is drawn.

An experimental sibling executable in `crates/keymap-overlay-winui` exercises a
pure-Rust WinUI 3 frontend through Microsoft's unreleased `windows-reactor`
crate. It calls the shared listener, reducer, model loader, and composer
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

On **Linux**, `src/ui/linux.rs` loads and validates the installed models, owns
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
optional GitHub CLI is present, verifies GitHub artifact attestations. Release
archives carry the MIT license and generated third-party license notices beside
the executable.

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
   `%USERPROFILE%/.config/keymap-overlay/`.
2. Builds the platform executable and installs it as
   `~/.config/keymap-overlay/keymap-overlay` on macOS and Linux, and as
   `%USERPROFILE%/.config/keymap-overlay/keymap-overlay.exe` on Windows.
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

Two things are specific to Windows. A running executable is locked there, so
the service is stopped before the binary is replaced rather than afterwards.
And the Run key is given no environment, so `KEYMAP_OVERLAY_LOG_DIR`
cannot be passed the way the plist and the unit pass it; the overlay falls back
to the same path under `USERPROFILE` instead, and `make install-overlay`
refuses to run if that variable was overridden.

The overlay writes logs to:

```text
~/.local/var/log/keymap-overlay/overlay.log
```

Logs rotate at 1 MiB and retain the current file plus three previous files.

`make uninstall-overlay` stops and removes the login service, installed binary,
and generated JSON models. It keeps the logs for troubleshooting.

## Firmware Workflow

```text
example/<keyboard>/keymap
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
the EEPROM-based workflow. This is optional: the default image-generation
path reads the keymap source compiled into the firmware.

### Shared Display Model

The Python generator converts QMK's keymap and keyboard JSON into one small,
versioned display model per layer. The model contains only canvas geometry,
labels, transparency metadata, held-state metadata, and encoder actions; it
contains no toolkit-specific objects and does not pass through keymap-drawer,
YAML, SVG, or another schema. All three platforms install these models as JSON,
compose the held layers in memory using QMK precedence, and render the result
with AppKit, GNOME Shell, Qt Quick, or WPF. Keys use quiet, nearly opaque fills
and a low-contrast
hairline so they stay distinct over bright and dark backgrounds; the held layer
key alone receives its pale tint. Display-only Unicode labels come from single-character
comments on `custom_keycodes` entries or an explicit `keymap-overlay-labels`
comment block in `keymap.c`. Platform blocks suffixed with `-macos`, `-linux`,
or `-windows` override common labels; `OVERLAY_PLATFORM` selects the target and
defaults to the current host. Encoder placement is the only project-specific geometry:
QMK knows the encoder count and pins but not where knobs sit, so `config.json`
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

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
  Linux: Qt Quick + KDE LayerShellQt
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

Device arrival notifications also select that usage. Linux receives them from
udev and macOS from `IOHIDManager`; either ends the current reader session so
all connected keyboards are enumerated again. This makes a reconnected
keyboard available even when another keyboard kept the previous session alive.

## Native Overlay

`crates/keymap-overlay` is one application with three platform windows. Reading
the keyboard, deciding what a report means, locating assets, and writing the
log are shared; only the window differs behind `src/ui/` and the Linux bridge.

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

The transparent WPF window is mapped once and shrinks to one pixel while idle.
Its HWND uses `WS_EX_NOACTIVATE`, `WS_EX_TOOLWINDOW`, and click-through styling,
so repeated layer presses cannot take focus and the overlay stays out of the
taskbar and Alt-Tab. `SetWindowPos` always includes `SWP_NOACTIVATE`.

Windows publishes one self-contained `keymap-overlay.exe`. The .NET single-file
bundle contains the Rust bridge DLL and extracts native content automatically
before launch; installation and autostart still manage one executable.

On **Linux**, `src/ui/qt.rs` sends reduced show/hide transitions over a local
Unix datagram socket. `QSocketNotifier` wakes the Qt main loop, so the resident
process remains event-driven without polling. The C++ side of
`keymap-overlay-qt-bridge` parses every installed JSON model at startup and
builds its keys, encoders, and labels as Qt Quick items.

The QML window uses KDE LayerShellQt's attached `Window` API. It requests the
overlay layer, no keyboard interaction, no exclusive zone, and placement on the
active screen. `Qt::WindowTransparentForInput` makes the surface click-through.
No plain Wayland application window can supply those semantics: the compositor
must grant the layer-surface role.

The bridge is one deliberately narrow exception to the workspace's
`unsafe_code = forbid` policy. CXX generates the unavoidable Rust/C++ FFI in
`keymap-overlay-qt-bridge`; the crate exposes one safe function, while the HID
protocol, transition state, socket ownership, and application lifecycle remain
in the forbid-unsafe Rust crates. Qt is linked into the same executable rather
than run as a helper process.

The model uses platform-independent point geometry. On macOS those values are
AppKit points, on Linux they are Qt logical pixels, and on Windows they are WPF
device-independent units. `PIXELS_PER_UNIT` controls the size of
one QMK layout unit. WPF interprets them as device-independent units and applies
the active monitor's DPI scale when positioning the native window.

### Requirements on Linux

- KDE Plasma on Wayland, Qt 6 Quick, and the KDE LayerShellQt QML module. GNOME,
  X11, and compositors without that QML integration are not currently
  supported.
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
2. Builds a release binary and installs it as
   `~/.config/keymap-overlay/keymap-overlay` on macOS and Linux, and as
   `%USERPROFILE%/.config/keymap-overlay/keymap-overlay.exe` on Windows.
3. Writes the per-user service definition:
   - macOS: the launchd agent
     `~/Library/LaunchAgents/com.sunaemon.keymap-overlay.plist`.
   - Linux: the systemd user unit
     `~/.config/systemd/user/keymap-overlay.service`, wanted by
     `graphical-session.target` because the overlay needs the compositor.
   - Windows: the current user's `KeymapOverlay` value under
     `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`.
4. Restarts the service so updates take effect immediately and starts it at
   future logins.

All three definitions start at login. macOS and Linux also restart after a
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
with AppKit, Qt Quick, or WPF. Keys use quiet, nearly opaque fills and a low-contrast
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
only those in-memory models, so they never leave the previous layer visible
while disk I/O completes.

Events already waiting when a UI loop wakes are reduced to their final active
layer before the window changes. Intermediate restores and switches are not
drawn on the way to a newer layer or a hide, and the macOS window swaps to an
empty content view while hiding so a later map cannot expose stale content.

### Native Windows Rather Than One Toolkit

Each system gets the window integration it can actually support. AppKit covers
macOS, Qt Quick plus KDE LayerShellQt covers Linux, and WPF covers Windows.
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

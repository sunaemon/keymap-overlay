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
platform-neutral geometry, labels, and state (one model per layer)
  ├─ macOS: <keyboard>_L<n>.json
  └─ Linux/Windows: Pillow → <keyboard>_L<n>.png
  ↓ make install-assets
platform configuration directory/<keyboard>_L<n>.<json|png>
```

`make install-assets` is the platform-independent model-generation and copy
target. It installs JSON on macOS and PNG on Linux. On Windows, generate PNGs
from WSL with `make install-assets`, then run the native `make install-overlay`;
WSL writes them directly to `%USERPROFILE%/.config/keymap-overlay/`.

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
  Windows: eframe/egui
  Linux: wlr-layer-shell surface, or an override-redirect X11 window
  ↓
matching <keyboard>_L<layer> asset is displayed
  ↓
matching layer key released
  ↓
previous held layer is restored, or the overlay is hidden when none remain
```

The overlay remains visible for the complete key hold. If momentary layers are
held together, it shows the most recently pressed one and restores the next
most-recent one still held on release; it hides once none remain.

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

`crates/keymap-overlay` is one Rust application with four windows. Reading the
keyboard, deciding what a report means, loading the asset, and writing the log
are shared; only the window differs, behind `src/ui/`.

On **macOS** (`src/ui/appkit.rs`) AppKit owns the complete view hierarchy. The
undecorated, always-on-top, click-through window uses an `NSGlassEffectView` as
its content. It parses the installed JSON at startup and builds each layer from
native `NSBox` and `NSTextField` views inside the glass view's `contentView`.
There is no rasterized key or label foreground on macOS. Hiding swaps in an
empty content view and shrinks the still-mapped window to one pixel, avoiding
native show animations on every layer-key press.

The application replaces the former Hammerspoon and Lua integration entirely.
No synthetic function-key events or Hammerspoon configuration are required.

On **Windows** (`src/ui/windows.rs`) the window is an eframe/egui window with
the same four properties, kept out of the taskbar and the alt-tab list. It
differs from every other backend in one way: **it is mapped once and never
hidden.**

Hiding it would take focus. `ViewportCommand::Visible(true)` becomes winit's
`WindowFlags::VISIBLE`, and winit issues `SW_SHOWNOACTIVATE` only for the first
show of a window built with `with_active(false)`; it then flips an internal
marker, so every later show is a plain `SW_SHOW`, which activates. Since the
overlay shows and hides on every key hold, from the second press onward the
window would take focus and swallow the keystrokes the layer key was held for —
the same failure described for X11 below. Nothing in winit 0.30 exposes
`WS_EX_NOACTIVATE` for an application window, and the workspace forbids unsafe,
so the style cannot be set by hand either.

So on Windows "hidden" means _drawing nothing_: the window keeps its place in
the stack, transparent and click-through, and hiding drops the texture and
shrinks the window, exactly as on macOS. Two
consequences are load-bearing. The clear colour must be fully transparent —
eframe's default is a translucent grey that no other backend ever shows,
because they all unmap, but here it would be a permanent rectangle across the
screen. And resizing must not activate either, which holds: winit's resize path
passes `SWP_NOACTIVATE`.

On **Linux** there are two windows, chosen at startup by `src/ui/linux.rs` and
overridable with `KEYMAP_OVERLAY_BACKEND` (`auto`, `layer-shell`, `x11`).

`src/ui/wayland.rs` is the one to want: a `zwlr_layer_shell_v1` surface on the
overlay layer, drawn into a `wl_shm` buffer. A Wayland application window has no
say over stacking and no way to be skipped by the pointer, so an ordinary window
cannot be this overlay; a layer surface can be both. Clicks pass through because
the surface's input region is empty.

`src/ui/x11.rs` is the fallback for compositors with no layer shell, GNOME above
all, reached through XWayland in a Wayland session and directly in an X11 one.
The window is **override-redirect**: the window manager does not manage it, so
it is never focused, never restacked below managed windows, and never decorated
or moved. That is not a detail. The usual route — asking for
`_NET_WM_STATE_ABOVE` — is a request a window manager may ignore, and it was
ignored in testing; worse, a managed window takes focus when it is mapped, which
here would mean swallowing the very keystrokes the layer key was held for. Being
unmanaged also means nothing places the window, so it is centred by hand. Clicks
pass through via `set_cursor_hittest`.

It uses winit and uploads pixels directly over X11 rather than using eframe,
because eframe offers no way to ask for an override-redirect window and because
blitting one decoded image per key hold does not need a GPU context. The direct
upload also provides the defined 32-bit ARGB format the transparent visual
requires; softbuffer's public pixel format has no alpha channel.

On Linux, hiding is unmapping: the overlay attaches a null buffer, which per the
protocol returns the layer surface to the state it had when it was created.
Showing a layer therefore re-sends the layer state, commits without a buffer,
and attaches the image when the configure that follows arrives. Switching
between visible layers of the same size instead replaces the buffer in one
commit, avoiding an unmap/configure round trip and a stale frame. The surface
is unmapped between key holds, so a hidden Linux overlay is not a window at all.
macOS and Windows instead keep a transparent, click-through window mapped to
avoid platform show behaviour that is inappropriate for the overlay.

The model uses platform-independent point geometry. On macOS those values are
AppKit points; on Linux and Windows the generated image is presented at its own
pixel size. `PIXELS_PER_UNIT` controls the size of one QMK layout unit. Windows
reports a scale factor that egui would otherwise apply on top, so that backend
pins `pixels_per_point` to 1.

### Requirements on Linux

- For the layer-shell window, a compositor that implements
  `zwlr_layer_shell_v1`: COSMIC, sway, Hyprland, wayfire and KDE Plasma do.
  Otherwise the X11 window is used, which needs XWayland in a Wayland session.
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
application. `make install-assets` builds keyboard-specific JSON or PNG assets
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
   install all layer assets (JSON on macOS, PNG on Linux). On Windows, verifies
   that WSL has already generated PNGs under
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
and generated JSON or PNG assets. It keeps the logs for troubleshooting.

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
labels, held-state flags, and encoder actions; it contains no toolkit-specific
objects and does not pass through keymap-drawer, YAML, SVG, or another schema.
macOS installs this model as JSON and renders it with AppKit. Linux and Windows
use the same in-memory model as input to the first-party Pillow compatibility
renderer, which supersamples and downsamples a transparent RGBA PNG for smooth
edges. Keys use quiet, borderless, nearly opaque fills so they stay distinct
over bright and dark backgrounds; the held layer key alone receives its pale
tint. Encoder placement is the only project-specific geometry:
QMK knows the encoder count and pins but not where knobs sit, so `config.json`
maps each encoder to its push-switch matrix position or to explicit `x`/`y`
layout coordinates. Matrix placement replaces the normal key drawing with one
circular knob that contains counter-clockwise, clockwise, and push actions.

The macOS runtime parses every installed JSON model and builds its native view
tree at startup. Linux and Windows decode and cache every installed PNG at
startup. Layer events therefore only select an in-memory view or image; they
never leave the previous layer visible while disk I/O or decoding completes.

Events already waiting when a UI loop wakes are reduced to their final active
layer before the window changes. Intermediate restores and switches are not
drawn on the way to a newer layer or a hide, and the macOS window swaps to an
empty content view while hiding so a later map cannot expose stale content.

### Four Windows Rather Than One Toolkit

eframe runs on Linux too, so the overlay could have had a single window
implementation. It would not have worked. On Wayland an application window
cannot raise itself above others or ignore the pointer, which is the entire
behaviour of this overlay, and only layer-shell offers both. On X11 eframe
cannot ask for an override-redirect window, so what it produces is a managed
window: measured taking focus every time it appeared, and never receiving the
always-on-top state it asked for.

So each system gets the window it can actually support. AppKit covers macOS,
eframe covers Windows, and keeping eframe off Linux also keeps egui, glutin and
accesskit out of that dependency tree.

Windows and macOS use different native-window implementations because macOS
can compose Liquid Glass and native controls directly while Windows needs the mapped,
non-activating behaviour described above.

The cost is four windows to maintain, each exercised only by the CI job for its
own system, and only one of the four — the layer surface — with real guarantees
behind it. What CI can prove is that each compiles and that the shared logic
passes; that a window stays on top, passes clicks through and never takes focus
has always needed a real machine.

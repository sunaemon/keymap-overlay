# Keymap Overlay Design

This project generates images for QMK keymap layers and displays the active
momentary layer in a native overlay, on macOS, Linux and Windows.

## Image Generation

```text
keymap.c (or VIAL EEPROM with VIAL=true)
  ↓ QMK c2json / Vitaly export
build/<keyboard>/qmk-keymap.json
  ↓ postprocess_qmk_keymap.py
build/<keyboard>/keymap-drawer.yaml
  ↓ keymap-drawer (one SVG per layer)
build/<keyboard>/<keyboard>_L<n>.svg
  ↓ resvg
build/<keyboard>/<keyboard>_L<n>.png
  ↓ make install-assets
platform configuration directory/<keyboard>_L<n>.png
```

`make install-assets` is the platform-independent image-generation and copy
target. On macOS and Linux, `make install-overlay` invokes it before installing
the application. On Windows, generate assets from WSL with `make
install-assets`, then run the native `make install-overlay`; WSL writes the
PNGs directly to `%USERPROFILE%/.config/keymap-overlay/`.

## Runtime Data Flow

```text
Momentary layer key held
  ↓
QMK Raw HID report (KMO protocol)
  ↓
Rust HID listener (hidapi)
  ↓
native transparent window
  macOS: eframe/egui
  Windows: eframe/egui
  Linux: wlr-layer-shell surface, or an override-redirect X11 window
  ↓
matching <keyboard>_L<layer>.png is displayed
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

## Native Overlay

`crates/keymap-overlay` is one Rust application with four windows. Reading the
keyboard, deciding what a report means, loading the image, and writing the log
are shared; only the window differs, behind `src/ui/`.

On **macOS** (`src/ui/eframe_window.rs`) the window is an eframe/egui window
that is undecorated, transparent, always-on-top and click-through. It is
explicitly hidden on its first frame to avoid a macOS visibility quirk, and
resized to the PNG dimensions immediately before it is shown.

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
the stack, transparent and click-through, and hiding drops the texture. Two
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

Hiding is unmapping: the overlay attaches a null buffer, which per the protocol
returns the layer surface to the state it had when it was created. Showing a
layer therefore re-sends the layer state, commits without a buffer, and attaches
the image when the configure that follows arrives. The surface is unmapped
between key holds, so a hidden overlay is not a window at all. That holds on
macOS and Linux; Windows is the exception described above.

The image is presented at its own pixel size on all three systems rather than
being scaled to the display; `DPI` in the Makefile is where an image is sized
for a screen. Windows reports a scale factor that egui would otherwise apply on
top, so that backend pins `pixels_per_point` to 1.

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
application. `make install-assets` builds keyboard-specific PNGs from the
source checkout. The platform installer then downloads the latest versioned
release archive, requires a matching entry in `SHA256SUMS`, and, when the
optional GitHub CLI is present, verifies GitHub artifact attestations. Release
archives carry the MIT license and generated third-party license notices beside
the executable.

The installers stop the running service before replacing its binary, preserve
the previous binary, notices and service definition until the new service
starts, and restore them if installation fails. Their uninstall modes remove
the executable, notices and login entry while retaining generated PNGs and logs.

Developers can instead use `make install-overlay`, which performs the following
source-build workflow:

`make install-overlay` performs the following steps:

1. On macOS and Linux, uses the `install-assets` target to generate and
   install all layer PNG assets. On Windows, verifies that WSL has already
   generated them under `%USERPROFILE%/.config/keymap-overlay/`.
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
and generated PNG assets. It keeps the logs for troubleshooting.

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

### PNG at Runtime

The runtime loads PNG files rather than SVGs. Rendering happens during the
build, leaving the overlay with a small and predictable image-loading path.

### Four Windows Rather Than One Toolkit

eframe runs on Linux too, so the overlay could have had a single window
implementation. It would not have worked. On Wayland an application window
cannot raise itself above others or ignore the pointer, which is the entire
behaviour of this overlay, and only layer-shell offers both. On X11 eframe
cannot ask for an override-redirect window, so what it produces is a managed
window: measured taking focus every time it appeared, and never receiving the
always-on-top state it asked for.

So each system gets the window it can actually support. eframe covers macOS and
Windows, where it works; keeping it off Linux also keeps egui, glutin and
accesskit out of that dependency tree.

Windows and macOS run the same toolkit but not the same file, because the one
thing that matters most differs between them: macOS hides the window between
key holds and Windows cannot, for the reasons above. Merging the two behind
`cfg` attributes would put that difference in the middle of every method rather
than in one place.

The cost is four windows to maintain, each exercised only by the CI job for its
own system, and only one of the four — the layer surface — with real guarantees
behind it. What CI can prove is that each compiles and that the shared logic
passes; that a window stays on top, passes clicks through and never takes focus
has always needed a real machine.

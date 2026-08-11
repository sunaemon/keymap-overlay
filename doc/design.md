# Keymap Overlay Design

This project generates images for QMK keymap layers and displays the active
momentary layer in a native overlay, on macOS and on Linux.

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
  ↓ make install-overlay
~/.config/keymap-overlay/<keyboard>_L<n>.png
```

`make install-overlay` runs the existing image-install workflow before it
installs the native application, so `make install` is not needed separately.

## Runtime Data Flow

```text
Momentary layer key held
  ↓
QMK Raw HID report (KMO protocol)
  ↓
Rust HID listener (hidapi)
  ↓
native transparent window
  macOS: eframe/egui        Linux: wlr-layer-shell surface
  ↓
matching <keyboard>_L<layer>.png is displayed
  ↓
matching layer key released
  ↓
overlay is hidden
```

The overlay remains visible for the complete key hold. It is hidden only when
the corresponding release report arrives.

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

`crates/keymap-overlay` is one Rust application with two windows. Reading the
keyboard, deciding what a report means, loading the image, and writing the log
are shared; only the window differs, behind `src/ui/`.

On **macOS** (`src/ui/macos.rs`) the window is an eframe/egui window that is
undecorated, transparent, always-on-top and click-through. It is explicitly
hidden on its first frame to avoid a macOS visibility quirk, and resized to the
PNG dimensions immediately before it is shown.

The application replaces the former Hammerspoon and Lua integration entirely.
No synthetic function-key events or Hammerspoon configuration are required.

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
between key holds, so a hidden overlay is not a window at all.

The image is presented at its own pixel size on both systems rather than being
scaled to the display; `DPI` in the Makefile is where an image is sized for a
screen.

### Requirements on Linux

- For the layer-shell window, a compositor that implements
  `zwlr_layer_shell_v1`: COSMIC, sway, Hyprland, wayfire and KDE Plasma do.
  Otherwise the X11 window is used, which needs XWayland in a Wayland session.
- Read access to the keyboard's `hidraw` node, which `make install-udev-rules`
  grants with a `uaccess` rule per keyboard. hidapi is built against hidraw
  rather than its default libusb backend: libusb would detach the kernel driver
  and stop the keyboard from typing, and only hidraw reports the usage page the
  Raw HID interface is selected by.

## Installation and Autostart

`make install-overlay` performs the following steps:

1. Generates and installs all layer PNG assets in `~/.config/keymap-overlay/`.
2. Builds a release binary and installs it as
   `~/.config/keymap-overlay/keymap-overlay`.
3. Writes the per-user service definition:
   - macOS: the launchd agent
     `~/Library/LaunchAgents/com.sunaemon.keymap-overlay.plist`.
   - Linux: the systemd user unit
     `~/.config/systemd/user/keymap-overlay.service`, wanted by
     `graphical-session.target` because the overlay needs the compositor.
4. Restarts the service so updates take effect immediately and starts it at
   future logins.

The two service definitions agree on behaviour: start at login, restart after a
crash, stay stopped after a clean exit, and pass the log directory in the
environment.

The overlay writes logs to:

```text
~/.local/var/log/keymap-overlay/overlay.log
```

Logs rotate at 1 MiB and retain the current file plus three previous files.

`make uninstall-overlay` stops and removes the launchd service, installed
binary, and generated PNG assets. It keeps the logs for troubleshooting.

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

## Design Decisions

### VIAL over VIA

The project uses VIAL because `vitaly` can read and write VIAL keymap data for
the EEPROM-based workflow. This is optional: the default image-generation
path reads the keymap source compiled into the firmware.

### PNG at Runtime

The runtime loads PNG files rather than SVGs. Rendering happens during the
build, leaving the overlay with a small and predictable image-loading path.

### Three Windows Rather Than One Toolkit

eframe runs on Linux too, so the overlay could have had a single window
implementation. It would not have worked. On Wayland an application window
cannot raise itself above others or ignore the pointer, which is the entire
behaviour of this overlay, and only layer-shell offers both. On X11 eframe
cannot ask for an override-redirect window, so what it produces is a managed
window: measured taking focus every time it appeared, and never receiving the
always-on-top state it asked for.

So each system gets the window it can actually support, and eframe stays on
macOS, where it works and where keeping it also keeps egui, glutin and accesskit
out of the Linux dependency tree.

The cost is three windows to maintain, each exercised only by the CI job for its
own system, and only one of the three — the layer surface — with real guarantees
behind it.

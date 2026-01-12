### End-to-End Data Flow

Image generation workflow:

```text
QMK keymap.c (or EEPROM when VIAL=true)
  ↓ generate data
build/qmk-keymap.json
  ↓ keymap parse
build/keymap-drawer.yaml
  ↓ keymap draw (per layer)
build/L0.svg, build/L1.svg, ...
  ↓ rsvg-convert
build/L0.png, build/L1.png, ...
  ↓ make install
~/.hammerspoon/L*.png
```

Firmware flash workflow:

```text
$(KEYBOARDS_DIR)/<keyboard>/keymap
  ↓ make flash
qmk flash
  ↓ firmware update
keyboard device
```

## Design Decisions

### VIAL vs VIA

While the original firmware for many supported keyboards uses VIA, this project prefers **VIAL**.

The choice of VIAL is driven by **vitaly**, a custom tool for interacting with the keyboard. VIAL provides a more open protocol that facilitates this integration. However, note the following trade-offs encountered during development:

- **Lack of Stable CLI:** Both VIA and VIAL lack a robust, stable command-line interface for all operations.
- **Manual Raw HID Handling:** To work around CLI limitations, the system often needs to handle Raw HID communication manually. This approach, while powerful, can be less maintainable than using a standardized, high-level CLI.

Despite these challenges, VIAL remains the preferred choice as it allows for the deep integration required by the `vitaly` workflow.

### Layer Change Detection

To avoid writing a custom HID driver, the overlay uses key sequences tied to unused
function keys (F13+) to signal layer changes. One downside is that macOS can map
F13/F14 to system actions (e.g., screen dim/brightness) on some setups, so those
mappings must be disabled manually. VIA/VIAL macros were explored but did not solve
this reliably, so the current approach relies on custom firmware behavior.

### Hammerspoon as Host

Hammerspoon was chosen for its robust macOS API access (event taps, screen info, canvas)
and because it is one of the most popular and well-supported open-source automation tools
for macOS, with a large community of power users who already use it for window management
or automation.

### Platform Focus

This project targets macOS because it is the author's primary environment. The QMK
processing pipeline is platform-independent, but the Hammerspoon overlay is macOS-specific.
Re-implementing the overlay on other platforms should be straightforward (e.g., AutoHotkey
on Windows or a small Python app on Linux).

### SVG vs PNG Rendering

Hammerspoon's image/canvas APIs do not render SVGs directly, and using an `hs.webview`
for SVG would add a heavier dependency and more Lua complexity. To keep the overlay
script simple and robust, this project converts SVGs to PNGs with `rsvg-convert`
and renders PNGs on a canvas.

### Keymap Drawer

This project uses `keymap-drawer` because it renders keymaps sufficiently well and
avoids reinventing the wheel with a custom renderer.

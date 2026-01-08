# QMK Keymap Hammerspoon Overlay

[![CI](https://github.com/sunaemon/keymap-overlay/actions/workflows/ci.yml/badge.svg)](https://github.com/sunaemon/keymap-overlay/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-green.svg)](LICENSE)
[![Keyboards: GPL-2.0-or-later](https://img.shields.io/badge/Keyboards-GPL--2.0--or--later-blue.svg)](keyboards/salicylic_acid3/insixty_en/LICENSE)
[![Platform: macOS](https://img.shields.io/badge/Platform-macOS-lightgrey.svg)](https://www.apple.com/macos/)

This repository provides tools to **generate keyboard layer images from QMK keymaps** and display them as an **on-screen overlay on macOS using Hammerspoon** while layer modifier keys are held.

## Overview

See `doc/design.md` for the end-to-end data flow and design notes.

This repo is set up for the `salicylic_acid3/insixty_en` keyboard, but the workflow should work for any QMK-compatible keyboard with minor configuration changes.

While holding a layer key on the keyboard, **Hammerspoon displays the matching layer image on screen**.

Set `KEYMAP_OVERLAY_DEBUG=1` to enable debug logging for the overlay.

## Getting Started

1. Clone this repository:

   ```bash
   git clone https://github.com/sunaemon/keymap-overlay.git
   cd keymap-overlay
   git submodule update --init --recursive
   ```

2. Install mise:

   See the [mise installation instructions](https://mise.jdx.dev/getting-started.html).

   If you are using Homebrew, you can install it with:

   ```bash
   brew install mise
   ```

3. Install Hammerspoon:

   See the [Hammerspoon installation instructions](https://www.hammerspoon.org/go/).

   If you are using Homebrew, you can install it with:

   ```bash
   brew install --cask hammerspoon
   ```

4. Set up the mise and UV environment:

   ```bash
   mise install
   mise exec -- uv sync --no-dev
   ```

5. Check setup (optional):

   ```bash
   make doctor
   ```

   `⚠ QMK home does not appear to be a Git repository! (no .git folder)` warnings can be ignored because the QMK firmware is included as a submodule in this setup.

6. Generate keymap images and install them to `~/.hammerspoon`:

   To use the keymap compiled in the firmware:

   ```bash
   make install
   ```

7. Flash firmware with the `layer-notify` keymap to your keyboard:

   ```bash
   make flash
   ```

### Use VIAL

1. To use the current keymap from your keyboard's EEPROM (requires VIAL-enabled firmware):

   ```bash
   make install VIAL=true
   ```

2. If only the keymap is updated, you can update it with VIAL:

   ```bash
   make flash-keymap
   ```

## License

The QMK keymap files located in `keyboards/` originate from @salicylic_acid3's fork of QMK firmware, licensed under the GNU General Public License v2.0 or later; see the [LICENSE](keyboards/salicylic_acid3/insixty_en/LICENSE) file in that directory for details.

The rest of this project is licensed under the MIT License. See the [LICENSE](LICENSE) file for details.

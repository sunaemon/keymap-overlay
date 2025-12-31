# QMK Keymap Hammerspoon Overlay

This repository provides tools to **generate keyboard layer images from QMK keymaps** and display them as an **on-screen overlay on macOS using Hammerspoon** while layer modifier keys are held.

## Overview

See `doc/design.md` for the end-to-end data flow and design notes.

While holding a layer key on the keyboard, **Hammerspoon displays the matching layer image on screen**.

Set `KEYMAP_OVERLAY_DEBUG=1` to enable debug logging for the overlay.

## Getting Started

1. Clone this repository:

   ```bash
   git clone https://github.com/sunaemon/keymap-overlay.git
   cd keymap-overlay
   git submodule update --init --recursive
   ```

2. Install Xcode command line tools:

   ```bash
   xcode-select --install
   ```

3. Install Homebrew if not already installed:

   ```bash
   /bin/bash -c "$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)"
   ```

4. Install required packages and tools:

   ```bash
   make setup
   ```

   This will install `mise` via Homebrew and set up all necessary tools and dependencies.

   If you are a developer and want to run lint/format, use:

   ```bash
   make setup-dev
   ```

5. Flash firmware with the layer-notification keymap to your keyboard.

   ```bash
   make flash
   ```

   If only the keymap is updated, you can update it with VIAL:

   ```bash
   make flash-keymap
   ```

6. Generate keymap images and install them to ~/.hammerspoon:

   To use the keymap compiled in the firmware:

   ```bash
   make install
   ```

   To use the current keymap from your keyboard's EEPROM (requires VIAL-enabled firmware):

   ```bash
   make install VIAL=true
   ```

## License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.

The QMK keymap files located in `keyboards/` originate from @salicylic_acid3's fork of QMK firmware, licensed under the GNU General Public License v2.0 or later; see the [LICENSE](keyboards/salicylic_acid3/insixty_en/LICENSE) file in that directory for details.

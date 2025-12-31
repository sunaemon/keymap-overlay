# Keymap Overlay Agent Guide

Welcome to the `keymap-overlay` project. This document provides an overview of the project's architecture, tools, and workflows to help you understand and contribute to the codebase.

## Project Overview

The goal of this project is to provide a seamless way to visualize QMK keyboard layers on macOS. It generates high-quality images (SVG/PNG) from your QMK keymap or VIA EEPROM and uses [Hammerspoon](https://www.hammerspoon.org/) to display these images as an on-screen overlay whenever you switch layers.

## Core Components

### 1. Keymap Data Generation (`scripts/`)

- `vitaly`: (External Tool) Reads/writes keymap data from a VIAL-enabled keyboard over HID.
- `count_layers.py`: Counts the number of layers in a QMK keymap JSON.
- `generate_key_to_layer.py`: Parses `keymap.c` to identify which keys trigger which layers.
- `generate_keycodes.py`: Scans QMK firmware for keycode definitions.
- `postprocess_keymap.py`: Cleans up and prepares the QMK keymap JSON for rendering (resolves transparency).
- `qmk_info_to_vial.py`: Converts QMK `keyboard.json` to VIAL-compatible `vial.json`.
- `sync_keycodes.py`: Synchronizes custom keycodes between `keymap.c` and configuration files.
- `update_vitaly_layout.py`: Merges QMK layer data into a VIAL configuration dump.
- `vitaly_to_qmk.py`: Converts a VIAL configuration dump back to QMK keymap JSON.

### 2. Visualization (`keymap-drawer`)

The project leverages [keymap-drawer](https://github.com/caksoylar/keymap-drawer) to transform QMK JSON data into stylized SVG and PNG images.

### 3. Hammerspoon Overlay (`keymap_overlay.lua`)

A Lua script that runs within Hammerspoon. It:

- Watches for layer change notifications from the keyboard (via custom raw HID or specific key sequences, depending on firmware implementation).
- Displays the corresponding PNG from `~/.hammerspoon/` on screen.

### 4. Firmware (`keyboards/` & `qmk_firmware/`)

- Contains local keyboard configurations and specialized keymaps (e.g., `layer-notify`) that facilitate communication with the Hammerspoon overlay.

## Tech Stack

- **Python**: Core logic for data extraction and processing.
  - `uv`: Modern Python package manager.
  - `keymap-drawer`: SVG generation.
  - `pydantic`: Data validation.
  - `typer`: CLI building.
- **Lua**: Hammerspoon integration for the UI overlay.
- **Makefile**: Orchestrates the entire build, installation, and flashing process.
- **QMK Firmware**: The underlying keyboard firmware.

## Key Workflows

### Installation

```bash
make setup        # Install system dependencies (brew)
make install      # Generate images and install to Hammerspoon
```

### Firmware Development

```bash
make flash        # Compile and flash QMK firmware to the keyboard
make patch-load   # Pull local keyboard changes into the QMK submodule
```

### Development & Linting

```bash
make lint         # Run ruff and ty
```

## Testing & Verification

Before running any verification steps, force a rebuild of generated artifacts:

```bash
make clean
```

### Firmware Compilation

To verify that the firmware compiles correctly with local changes:

```bash
make compile
```

This target validates the `keyboard.json` structure, generates the VIAL-compatible JSON, and executes `qmk compile`.

### Keymap Flashing (VIAL/Vitaly)

To verify that the keymap can be fetched, merged, and uploaded back to a VIAL-enabled device:

```bash
make flash-keymap
```

This workflow:

1. Dumps the current configuration from the device using `vitaly`.
2. Merges the QMK keymap into the dumped configuration.
3. Loads the updated configuration back to the device.

### Application Installation

To verify the full installation process (generating images and installing to Hammerspoon):

```bash
make install            # Build from source and install
make install VIAL=true  # Build from VIAL EEPROM and install
```

These commands:

1. Generate the required JSON and image assets in `build/`.
2. Copy the Lua overlay, generated PNGs, and layer mapping to `~/.hammerspoon/`.
3. Update `~/.hammerspoon/init.lua` to include the overlay.

## Coding Standards

### Python

- Use `pathlib.Path` for all path manipulations. Do not use string concatenation or `os.path`.
- Use `Typer` for all CLI scripts.
- Use `logger.info` for status messages, `logger.warning` for non-fatal issues, `logger.error` for recoverable errors, `logger.exception` inside `except` blocks to include stack traces, and `logger.critical` for fatal errors. Configure the logger via `src.util.get_logger`. The default log level is set to `INFO`.
- Use modern type hints (Python 3.10+):
  - Use `| None` instead of `Optional[T]`.
  - Use built-in generic types like `dict` and `list` instead of `Dict` and `List` from the `typing` module.
- Use Pydantic validation at runtime (e.g., `TypeAdapter.validate_python`, `model_validate`) and avoid `typing.cast`.

### Error Handling Policy

- In library helpers, raise specific exceptions; avoid `sys.exit` outside of CLI entrypoints.
- In `@app.command()` functions, catch errors, log with `logger.exception(...)`, and exit with `raise typer.Exit(code=1)`.
- Prefer `OSError` for filesystem failures; include the path in the log message.
- Avoid `print` for errors; use logging so stderr output stays consistent.

## Directory Structure

- `keyboards/`: Local keyboard-specific configurations and keymaps.
- `scripts/`: Python utility scripts.
- `typings/`: Type stubs for Python libraries.
- `build/`: Temporary directory for generated artifacts (JSON, SVG, PNG).
- `qmk_firmware/`: The QMK firmware repository.

## Important Files

- `Makefile`: The primary entry point for all automation.
- `keymap_overlay.lua`: The Lua component installed to Hammerspoon.
- `pyproject.toml`: Python dependencies and tool configurations.

# Keymap Overlay Agent Guide

Welcome to the `keymap-overlay` project. This document covers the architecture,
tools, and conventions you need to contribute to the codebase.

Read `doc/design.md` first: it is the authoritative description of the data
flow, the Raw HID protocol, and the installation model. This guide describes
how to work in the repository, not how the system is designed.

## Project Overview

The project renders one image per QMK keymap layer and displays the matching
image in a native overlay while a momentary layer key is held, on macOS, Linux
and Windows. The keyboard reports layer presses over Raw HID; a Rust
application listens for those reports and shows the image.

Firmware is the exception: it is not compiled or flashed on Windows, and the
targets that would do so stop with a message pointing at WSL, macOS or Linux.

There are three parts:

1. **Python scripts** (`scripts/`, `src/`) that turn a QMK keymap into the JSON
   `keymap-drawer` needs, and that push keymaps to VIAL devices.
2. **Rust crates** (`crates/`) that implement the Raw HID protocol and the
   overlay window.
3. **Firmware glue** (`firmware/`, `example/`) that sends the Raw HID reports.

## Core Components

### 1. Keymap Data Generation (`scripts/`)

- `count_layers.py`: Counts the number of layers in a QMK keymap JSON.
- `generate_keycodes.py`: Scans QMK firmware for keycode definitions.
- `generate_custom_keycodes.py`: Extracts the `custom_keycodes` enum from
  `keymap.c` and assigns each entry its numeric value.
- `postprocess_qmk_keymap.py`: Prepares the QMK keymap JSON for **drawing**.
  Applies custom keycode names and resolves `KC_TRNS`.
- `generate_vial.py`: Converts QMK `keyboard.json` to a VIAL `vial.json`.
- `generate_vitaly_layout.py`: Merges a QMK keymap into a VIAL dump for
  flashing.
- `generate_qmk_keymap_from_vitaly.py`: Converts a VIAL dump back to QMK keymap
  JSON (the `VIAL=true` path).
- `mount_uf2_volume.py`: Linux only. Waits for the `RPI-RP2` volume an rp2040
  bootloader exposes and mounts it, with `sudo`, where `qmk` looks for it.
- `vitaly`: external tool that reads and writes VIAL keymaps over HID.

Transparency resolution is for **display only**. `KC_TRNS` must survive intact
on the path that writes to the device, otherwise layers stop inheriting from
layer 0 in EEPROM. `flash-keymap` therefore consumes the _raw_ keymap JSON,
never `$(QMK_KEYMAP_JSON)`.

### 2. Visualization (`keymap-drawer`)

The project uses [keymap-drawer](https://github.com/caksoylar/keymap-drawer) to
turn QMK JSON into SVGs, then `resvg` to rasterize them to PNG.

### 3. Rust Crates (`crates/`)

- `keymap-core`: the Raw HID wire format (`parse_raw_layer_event`) and its
  tests. Pure logic, no I/O, so it stays unit-testable.
- `keymap-overlay`: the overlay binary. Listens for Raw HID reports on a
  background thread and shows `<keyboard_id>_L<layer>.png` in a transparent,
  always-on-top, click-through window.

`main.rs` holds everything the systems share — the listener, the transitions,
image loading, the rotating log — and `src/ui/` holds the one thing they cannot
share:

- `ui/eframe_window.rs`: the macOS window, an eframe/egui window. Layer events
  are drained in `App::logic`, not `App::ui`: eframe runs no egui pass while the
  viewport is hidden, which is where this overlay sits between key holds, so
  work put in `ui` would never run when the press that shows it arrives.
- `ui/windows.rs`: the Windows window, also eframe/egui, but mapped once and
  never hidden. Hiding it would take focus — winit gives a window only one
  non-activating show — so "hidden" there means drawing nothing, and the clear
  colour has to be fully transparent rather than eframe's translucent default.
  Read the module comment before changing anything about when it shows.
- `ui/wayland.rs`: a `zwlr_layer_shell_v1` surface drawn into a `wl_shm` buffer.
  A plain Wayland window cannot stay on top or pass clicks through, so this is
  not a portability nicety; it is the only way the overlay works there.
- `ui/x11.rs`: an override-redirect winit window drawn directly over X11, for
  compositors with no layer shell (GNOME) and for X11 sessions. Override-redirect
  rather than `_NET_WM_STATE_ABOVE` because a managed window takes focus when it
  is mapped, which would swallow the keystrokes the layer key was held for; see
  `doc/design.md`.
- `ui/linux.rs`: picks between the last two, honouring `KEYMAP_OVERLAY_BACKEND`.

Cargo gates the dependencies per target, which is why eframe is kept off Linux
and hidapi uses hidraw there rather than its default libusb backend, and its
own `windows-native` backend on Windows. Keep new dependencies on the same side
of that line as the code using them.

The overlay is event-driven. Delivering an event also wakes the UI thread —
`request_repaint()` on macOS, a calloop channel on Linux, behind the
`LayerEventSink` trait — so there is no polling loop and no periodic repaint.
Do not reintroduce one: this process runs from login to logout, so idle cost
matters.

### 4. Firmware (`firmware/`, `example/`)

`firmware/layer_notify.h` is the shared header copied into the QMK keymap at
build time. It owns both the report format and the momentary-layer detection
(`keymap_overlay_notify_momentary_layer`). Keymaps call it from
`process_record_user` and must still return `true` so QMK performs the layer
switch itself.

Only momentary (`MO`) layers are reported. `TO`/`TG`/`LT`/`LM` are deliberately
ignored: the overlay hides on the matching release, and a layer that stays on
would leave it on screen indefinitely.

`KEYBOARD_ID` identifies a keyboard end to end — it is a directory name under
`$(KEYBOARDS_DIR)`, a `-DKEYBOARD_ID` macro in the firmware, one byte in the
Raw HID report, and the prefix of the installed PNGs. It must be an integer
between 0 and 255; the Makefile and `layer_notify.h` both enforce this.

## Tech Stack

- **Python**: keymap data extraction and processing.
  - `uv`: package manager. `pydantic` for validation, `typer` for CLIs.
- **Rust**: the overlay (`eframe`/`egui`, `hidapi`).
- **Makefile**: orchestrates build, installation, and flashing.
- **mise**: pins every tool version, including formatters and linters.
- **lefthook**: manages the git hooks declared in `lefthook.yml`.
- **QMK Firmware**: the keyboard firmware, vendored as a submodule.

## Key Workflows

### Installation

```bash
make setup              # Install system dependencies and toolchains
make install-udev-rules # Linux only: grant Raw HID access to the login user
make install-assets     # Generate and copy layer PNG assets
make install-overlay    # Build and install the login service
```

`make setup` and everything that installs or starts the overlay dispatch on
`OS_FAMILY`, derived from `uname -s` at the top of the Makefile — `windows`
covers the `MINGW*` and `MSYS*` that an MSYS2 or Git Bash shell reports. A
target that differs between the systems gets an `_<action>_$(OS_FAMILY)` helper
rather than a shell conditional inside one recipe.

Windows adds one such helper the other two do not need: `_stop_service_windows`,
because a running executable is locked there and has to go before the binary is
replaced. Its macOS and Linux siblings are deliberately empty.

### Firmware Development

```bash
make compile      # Validate keyboard.json, generate vial.json, run qmk compile
make flash        # Compile and flash QMK firmware (needs KEYBOARD_ID)
make patch-load   # Pull local keyboard changes back out of the QMK submodule
```

`flash` dispatches to `_flash_$(OS_FAMILY)`. The Linux side first mounts the
UF2 bootloader volume for the bootloaders that expose one, because `qmk` only
deploys to a volume something else already mounted; see `mount_uf2_volume.py`.
It reads `BOOTLOADER` out of `keyboard.json` the same lazy way as `DEVICE_PID`,
so targets that never flash do not pay for the lookup.

The overlay build shell on Windows is MSYS2 or Git Bash, not QMK MSYS, so its
`make compile`, `make flash`, and `make flash-keymap` targets deliberately stop
there. This does not prevent manual flashing of an already-built `.uf2`: put
the keyboard in its bootloader and copy the file onto the mounted `RPI-RP2`
volume in Explorer.

### Development & Verification

```bash
make format       # Format everything (ruff, cargo fmt, mbake, prettier, taplo, clang-format)
make lint         # ruff check, ty, cargo clippy -D warnings
make test         # pytest
make test-rust    # cargo test --workspace
make build-overlay # cargo build --release -p keymap-overlay
make audit        # cargo-audit against the RustSec advisory database
```

Force a rebuild of generated artifacts with `make clean` before verifying
anything that depends on `build/`.

CI runs four jobs. On Linux it runs `lint`, `format`, `test`, `test-rust` and
`build-overlay`, then fails if formatting produced a diff, so anything you add
must survive `make format` unchanged. On macOS and on Windows it runs `test`,
`test-rust` and `build-overlay`; the Windows job sets `shell: bash` so the
Makefile runs under Git Bash. Each job builds only its own windows, so a change
to `ui/eframe_window.rs` is compiled by the macOS job alone, `ui/windows.rs` by
the Windows job alone, and `ui/wayland.rs` or `ui/x11.rs` by the Linux job. A
fourth job runs `make audit`.

Only the Linux job lints, so Windows-only code is never seen by clippy in CI.
Run `cargo clippy --target x86_64-pc-windows-msvc -p keymap-overlay -- -D warnings`
by hand after touching `ui/windows.rs`; `cargo check` for that target works
from macOS and Linux too, and catches most of what CI would.

`make audit` is the only check that can start failing without anything here
changing, because the RustSec database grows on its own. Advisories that are
deliberately ignored, and why, live in `.cargo/audit.toml` — read the reason
before adding another one. Dependabot proposes updates for `cargo`, `uv`, and
GitHub Actions weekly; the tool versions pinned in `mise.toml` and
`mise.dev.toml` are not covered by it and still need bumping by hand.

`make install-overlay` still cannot be exercised in CI, because `launchctl
bootstrap` and `systemctl --user` need a real login session, and the layer-shell
window needs a running compositor. Changes to the plist, the systemd unit, the
Windows Run key, the udev rules, or the window itself have to be verified by
hand.

On Windows the check that matters most is that the overlay never takes focus:
type into a text editor, hold a layer key while continuing to type, and confirm
every keystroke lands and the caret does not move. Repeat it — the failure mode
this backend is built around only appears from the _second_ show onward.

### Git Hooks

`lefthook` manages the hooks, and `make setup` installs them. Run
`make install-hooks` on its own if they are missing, `make uninstall-hooks` to
remove them.

| Hook         | Runs                          |
| ------------ | ----------------------------- |
| `pre-commit` | `make format`, `make lint`    |
| `commit-msg` | `make check-commit-message`   |
| `pre-push`   | `make test`, `make test-rust` |

`pre-commit` restages only files that were already staged, so a commit never
picks up unrelated files that `make format` happened to touch. `pre-push`
skips when there is nothing new to push.

Commit subjects must be at most 72 characters and must not end with a period,
and a body must be separated from its subject by a blank line. The subjects git
writes itself (`Merge `, `Revert `, `fixup!`, `squash!`) are exempt. The rules
live in `scripts/check_commit_message.py` and are tested like any other script.

Set `LEFTHOOK=0` to skip hooks for a single command, e.g. `LEFTHOOK=0 git commit`.

### Worktrees

Every worktree belongs under `.claude/worktrees/<name>`, and nowhere else:

```bash
git worktree add .claude/worktrees/<name> -b <branch>
```

`.claude/worktrees/` is gitignored, so worktrees kept there never surface as
untracked files, and they stay next to the checkout they came from instead of
in some sibling directory.

A new worktree starts with its submodules empty: run
`git submodule update --init --recursive` inside it before building or
flashing, otherwise `qmk_firmware/` is missing. `build/`, `target/` and
`.venv/` are not shared between worktrees either, so the first build in one is
a cold build.

Remove a worktree with `git worktree remove .claude/worktrees/<name>` rather
than deleting the directory, so its administrative files go too.

### Keymap Flashing (VIAL/Vitaly)

```bash
make flash-keymap   # Parse keymap.c and write the keymap to EEPROM
```

Dumps the device configuration with `vitaly`, merges the QMK keymap into it,
and loads it back. Iterates all keyboards unless `KEYBOARD_ID` is set. It
rejects `VIAL=true`, which would read the device and write it straight back.

## Coding Standards

### Ordering (Top-Down)

Prefer Top-Down ordering for all code changes:

1. Imports/includes
2. Module/file constants
3. Public API (highest-level workflow first)
4. Private helpers in the order they are used by the public API
5. Entrypoints/registration blocks (e.g., `if __name__ == "__main__"`, exports) at the end

Note: CLI `main` functions are part of the public API and should appear with other public functions, not at the very end.

### Docstrings

Use one-line triple-quoted docstrings for functions and classes, e.g.:

`"""Returns a logger instance with the given name."""`

### Python

- Use `pathlib.Path` for all path manipulations. Do not use string concatenation or `os.path`.
- Use `Typer` for all CLI scripts.
- Scripts are internal; invoke them via `python -m scripts.<name>` from the Makefile and do not add `[project.scripts]` entrypoints.
- Prefer `Annotated[...]` style for Typer CLI parameters (e.g., `Annotated[Path, typer.Option(...)]`) for consistent typing and CLI metadata.
- Use `logger.info` for status messages, `logger.warning` for non-fatal issues, `logger.error` for recoverable errors, `logger.exception` inside `except` blocks to include stack traces, and `logger.critical` for fatal errors. Initialize logging in CLI entrypoints with `src.util.initialize_logging()`. The default log level is `INFO`.
- Use modern type hints (Python 3.10+):
  - Use `| None` instead of `Optional[T]`.
  - Use built-in generic types like `dict` and `list` instead of `Dict` and `List` from the `typing` module.
- Use Pydantic validation at runtime (e.g., `TypeAdapter.validate_python`, `model_validate`) and avoid `typing.cast`.

#### Error Handling Policy

- In library helpers, raise specific exceptions; avoid `sys.exit` outside of CLI entrypoints.
- In `@app.command()` functions, catch errors, log with `logger.exception(...)`, and exit with `raise typer.Exit(code=1)`.
- Prefer `OSError` for filesystem failures; include the path in the log message.
- Avoid `print` for errors; use logging so stderr output stays consistent.

### Rust

- The workspace forbids `unsafe_code` and denies clippy warnings; keep it that way.
- Keep protocol and other pure logic in `keymap-core` where it can be tested
  without hardware, and confine I/O to `keymap-overlay`.
- Use `anyhow::Context` to attach the path or device to an error rather than
  logging and discarding it.

## Directory Structure

- `crates/`: Rust workspace (`keymap-core`, `keymap-overlay`); the overlay's
  per-system windows live in `crates/keymap-overlay/src/ui/`.
- `firmware/`: Shared QMK header copied into keymaps at build time.
- `example/`: Local keyboard configurations and keymaps, one numbered directory
  per keyboard (configurable via `KEYBOARDS_DIR`).
- `scripts/`: Python utility scripts.
- `src/`: Shared Python models (`types.py`) and helpers (`util.py`).
- `typings/`: Type stubs for Python libraries.
- `tests/`: pytest suite and its JSON fixtures.
- `doc/`: Design documentation and README images.
- `build/`: Generated artifacts (JSON, SVG, PNG). Not checked in.
- `qmk_firmware/`: The QMK firmware submodule.

## Important Files

- `Makefile`: The primary entry point for all automation.
- `doc/design.md`: Data flow, Raw HID protocol, and installation model.
- `lefthook.yml`: The git hooks and the make targets they run.
- `mise.toml`: Pinned tool versions and the `format`/`lint` tasks.
- `firmware/layer_notify.h`: Raw HID report construction and MO detection.
- `pyproject.toml`: Python dependencies and tool configurations.

# Keymap Overlay Agent Guide

Welcome to the `keymap-overlay` project. This document covers the architecture,
tools, and conventions you need to contribute to the codebase.

Read `docs/design.md` first: it is the authoritative description of the data
flow, the Raw HID protocol, and the installation model. This guide describes
how to work in the repository, not how the system is designed.

## Project Overview

The project renders one display model per QMK keymap layer and displays the
matching model in a native overlay while a momentary layer key is held, on
macOS, Linux and Windows. The keyboard reports layer presses over Raw HID; a
Rust application listens for those reports and updates the window.

Firmware is the exception: it is not compiled or flashed on Windows, and the
targets that would do so stop with a message pointing at WSL, macOS or Linux.

There are four parts:

1. **Firmware metadata tooling** (`model/`) that generates the self-describing
   Vial definition compiled into each keyboard.
2. **Native display-model library** (`overlay/keymap-overlay-generator`) that
   the runtime links, reading a connected device directly — no Python involved
   on this path.
3. **Native overlay** (`overlay/`) that implements the Raw HID protocol,
   native macOS window, Linux D-Bus daemon and
   Qt client, and Windows bridge.
4. **Firmware glue** (`firmware/`) that sends the Raw HID reports.

## Core Components

### 1. Native Display Model Generation (`overlay/keymap-overlay-generator`)

The runtime's startup-read path. A root Cargo
workspace member whose library is linked into `keymap-overlay-runtime`; no
separate generator executable is installed or shipped. Its first-party Vial
client reads a connected device's embedded Vial definition and dynamic keymap
in one Raw HID session. `device.rs` resolves custom keycodes directly, while
`model.rs` owns geometry and model construction, `labels.rs` owns generic and
platform-specific presentation labels, and `types.rs` mirrors `model/src/types.py`'s
`keyboard.json`/`config.json` structs.

### 2. Keymap Data Generation (`model/`)

Used to build firmware metadata and for standalone keymap conversion tools.

- `generate_vial.py`: Converts QMK `keyboard.json` to a VIAL `vial.json`,
  embedding each `keymap.c` custom keycode's name and display glyph so the
  compiled firmware is self-describing.
- `generate_vitaly_layout.py`: Merges a QMK keymap into a VIAL dump for
  flashing.
- `generate_qmk_keymap_from_vitaly.py`: Converts a VIAL dump back to QMK
  keymap JSON as a standalone conversion tool.

Transparency resolution is for **display only**. `KC_TRNS` must survive intact
in the compiled firmware, otherwise layers stop inheriting from layer 0 in
EEPROM. Only the overlay resolves transparency, in memory.

### 3. Visualization

The generator produces a platform-neutral model from QMK `keyboard.json`;
encoder positions come from each keyboard's project `config.json`, because QMK
describes encoder pins but not their physical layout. All three systems
install JSON and draw the model with AppKit, GNOME Shell, Qt Quick, or WPF. An
encoder placed at a matrix position replaces that push key with one circular
control showing counter-clockwise, clockwise, and push actions.

### 4. Native Overlay (`overlay/`)

- `overlay/keymap-core`: the Raw HID wire format (`parse_raw_layer_event`) and
  active-layer state reducer. Pure logic, no I/O, so it stays unit-testable.
- `overlay/keymap-overlay-runtime`: the shared listener, overlay transition
  adapter, model composition, logging code, and frontend-facing library API.
- `overlay/platforms/macos/appkit`: the native macOS executable and
  IOHIDManager arrival watcher.
- `overlay/platforms/linux/daemon`: the Linux HID and D-Bus service executable.
- `overlay/platforms/linux/protocol`: the typed Linux renderer D-Bus contract
  implemented by the daemon and consumed by both native renderers.
- `overlay/platforms/windows/bridge`: the audited Windows C ABI boundary. WPF
  supplies a wake callback and takes the final reduced show/hide transition.

The core library owns protocol and active-layer state semantics. The runtime
library holds the shared listener, overlay transitions, model composition, and
rotating log, while each platform owns its
executable and integration code:

- `overlay/platforms/macos/appkit`: the native macOS AppKit window and semantic
  JSON renderer.
- `overlay/platforms/windows/wpf`: the native WPF window. Win32 styles make it
  transparent, click-through, topmost, and non-activating. A self-contained
  single-file publish embeds the Rust bridge DLL for automatic extraction.
- `overlay/platforms/linux/daemon`: reduces HID events, loads the final semantic model, and
  publishes renderer state over session D-Bus. The GNOME Shell extension
  consumes it directly; the separate `keymap-overlay-qt` client receives it
  through QtDBus. KDE LayerShellQt supplies the Wayland overlay surface outside
  GNOME.

Cargo gates the Rust dependencies per target, the standalone Qt client is built
only on Linux, the C ABI bridge is kept on Windows, and hidapi uses hidraw on
Linux rather than its default libusb backend. Keep new dependencies on the same
side of that line as the code using them.

The overlay is event-driven. Delivering an event also wakes the UI thread — an
AppKit channel on macOS, a QtDBus or Shell signal callback on Linux, or a WPF
dispatcher callback on Windows, behind the `LayerEventSink` trait — so there is
no polling loop and no periodic repaint.
Do not reintroduce one: this process runs from login to logout, so idle cost
matters.

### 5. Firmware (`firmware/`, `firmware/examples/`)

`firmware/layer_notify.h` is the shared header copied into the QMK keymap at
build time. It owns both the report format and the momentary-layer detection
(`keymap_overlay_notify_momentary_layer`). Keymaps call it from
`process_record_user` and must still return `true` so QMK performs the layer
switch itself.

`firmware/tools/mount_uf2_volume.py` is Linux-only. It waits for the
`RPI-RP2` volume an rp2040 bootloader exposes and mounts it, with `sudo`, where
`qmk` looks for it.

Only momentary (`MO`) layers are reported. `TO`/`TG`/`LT`/`LM` are deliberately
ignored: the overlay hides on the matching release, and a layer that stays on
would leave it on screen indefinitely.

`KEYBOARD_ID` identifies a keyboard end to end — it is a directory name under
`$(KEYBOARDS_DIR)`, a `-DKEYBOARD_ID` macro in the firmware, one byte in the
Raw HID report, and the prefix of the installed layer assets. It must be an integer
between 0 and 255; the Makefile and `layer_notify.h` both enforce this.

## Tech Stack

- **Python**: firmware metadata generation and standalone keymap conversions.
  - `uv`: package manager. `pydantic` provides validation and `typer` the CLIs.
- **Rust/C++/C#/GJS**: the overlay (AppKit on macOS, WPF on Windows, GNOME
  Shell or Qt Quick plus KDE LayerShellQt on Linux, and `hidapi`), and the
  native display-model library (`overlay/keymap-overlay-generator`).
- **Makefile**: orchestrates build, installation, and flashing.
- **mise**: pins every tool version, including formatters and linters.
- **lefthook**: manages the git hooks declared in `lefthook.yml`.
- **QMK Firmware**: the keyboard firmware, vendored as a submodule.

## Key Workflows

### Installation

```bash
make setup              # Install system dependencies and toolchains
make install-udev-rules # Linux only: grant Raw HID access to the login user
make install-overlay    # Build and install the login service
```

The running overlay reads every connected keyboard model into memory at
startup (Startup Refresh in `docs/design.md`). A disconnected keyboard has no
model until the process is restarted with it connected. Release archives ship
neither keyboard definitions nor model caches.

`make setup` and everything that installs or starts the overlay dispatch on
`OS_FAMILY`, derived from `uname -s` at the top of the Makefile — `windows`
covers the `MINGW*` and `MSYS*` that MSYS2 UCRT64 and the Windows CI shell
report. A target that differs between the systems gets an
`_<action>_$(OS_FAMILY)` helper rather than a shell conditional inside one
recipe.

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

The overlay build shell on Windows is MSYS2 UCRT64, not QMK MSYS, so
`make compile`, `make flash`, and other QMK-backed targets deliberately stop
there. Native Raw HID startup still reads the device. This does not
prevent manual flashing of an already-built `.uf2`: put the keyboard in its
bootloader and copy the file onto the mounted `RPI-RP2` volume in Explorer.

### Development & Verification

```bash
make format       # Format everything (ruff, cargo fmt, mbake, prettier, taplo, clang-format)
make lint         # ruff check, ty, cargo clippy -D warnings
make test         # pytest
make test-rust    # cargo test --workspace
make test-installer-sh # release installer integration tests with stubbed services
make build-overlay # build the release overlay for the current platform
make test-release-acceptance-macos # macOS release gate: installer rollback + AppKit E2E
make test-release-acceptance-linux # Linux release gate: installer rollback + both E2E halves
make audit        # cargo-audit against the RustSec advisory database
make check-licenses # verify release third-party license notices without changing them
make licenses     # regenerate release third-party license notices
```

The installed hooks own the common local verification gates. When publishing a
change, do not run `make format`, `make lint`, `make test`, or `make test-rust`
immediately before committing or pushing only to repeat them: pre-commit runs
format and lint, while pre-push runs both test suites. During implementation,
run the smallest relevant targeted checks. In a pull request's verification
section, report the hook results once and list only change-specific checks
separately.

### Hardware Release Gate

When preparing, reviewing, merging, or publishing a release, read and follow
`docs/releasing.md`, `docs/hardware-release-testing.md`, and
`docs/release-test-coverage.md`. `CLAUDE.md` links to this guide, so these
instructions are shared by Codex and Claude. Use
`.github/PULL_REQUEST_TEMPLATE/release.md` for the version-bump PR. After
automated build and test checks pass, complete the hardware gate for the exact
PR head SHA before merge.

Never infer a physical pass from simulation or CI. Mark an `HRG` item complete
only from an observed physical run or explicit tester evidence. A new
behavior-affecting commit invalidates affected results. Do not merge the
release PR, create a tag, publish a release, weaken the check, or bypass a
failing `hardware-release-gate`. If required hardware is unavailable, stop and
ask the release owner to complete the physical checks.

`make check-licenses` regenerates the third-party notice in a temporary
directory and compares it without changing the worktree. Pre-commit runs it
only when staged Cargo dependency or package metadata, cargo-about tooling,
license rendering configuration, or the notice itself can affect the result.
Run `make licenses` and stage the generated notice when that check reports
drift.

Force a rebuild of generated artifacts with `make clean` before verifying
anything that depends on `build/`.

CI has `linux-x86_64` and `linux-arm64` jobs, each running `lint`, `format`,
`test`, `test-rust`, `check-licenses`, and `test-release-acceptance-linux`
(installer rollback plus both E2E halves against the release overlay), then
failing if formatting or linting produced a diff. The `mac-arm64` job runs `test`,
`test-rust`, and `test-release-acceptance-macos` (installer rollback plus the
simulated AppKit E2E test). On Windows it runs `test`, the `install.ps1` Pester
suite, `test-rust` and `build-overlay`; the other Windows steps set
`shell: bash` so the Makefile runs under Git Bash. Each job builds only its own
native backend, so `ui/appkit.rs` is compiled by the macOS job, WPF and its Rust
bridge by the Windows job, and the D-Bus daemon and Qt renderer by the Linux
job. A fourth job runs `make audit`.

Only the Linux job runs the complete lint task. After changing the Windows
bridge, run clippy against its manifest; after changing WPF, publish the
self-contained project on Windows. The Windows CI job performs both builds.

`make audit` is the only check that can start failing without anything here
changing, because the RustSec database grows on its own. Advisories that are
deliberately ignored, and why, live in `.cargo/audit.toml` — read the reason
before adding another one. Dependabot proposes updates for `cargo`, `uv`, and
GitHub Actions weekly; the tool versions pinned in `mise.toml` and
`mise.dev.toml` are not covered by it and still need bumping by hand.

When Codex invokes the GitHub CLI (`gh`), run it outside the sandbox so it can
access the user's GitHub authentication and network connection.

`make install-overlay` still cannot be exercised in CI, because `launchctl
bootstrap` and `systemctl --user` need a real login session, and the layer-shell
window needs a running compositor. The release-installer tests cover generated
service files, upgrades, rollback, and uninstall with temporary homes and
stubbed service commands, but changes to actual service registration, the udev
rules, or the window itself still have to be verified by hand.

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
live in `tools/check_commit_message.py` and are tested like any other script.

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
`make setup-firmware` inside it before building or flashing. This initializes
the Vial QMK checkout and only the ChibiOS, LUFA, and Pico SDK dependencies
required by the tracked keyboards; a recursive update downloads unrelated MCU
and UI libraries. Adding an unknown processor warns and safely falls back to a
recursive update. `build/`, `target/` and
`.venv/` are not shared between worktrees either, so the first build in one is
a cold build.

Remove a worktree with `git worktree remove .claude/worktrees/<name>` rather
than deleting the directory, so its administrative files go too.

### Vial EEPROM Ownership

`make flash` compiles with a fresh EEPROM epoch. On first boot after flashing,
the firmware resets QMK and Vial EEPROM and initializes the dynamic keymap from
the compiled `keymap.c`. Ordinary restarts retain subsequent Vial edits. The
runtime only reads Vial state; it never writes the keyboard.

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

Use one-line triple-quoted docstrings for Python functions and classes, e.g.:

`"""Returns a logger instance with the given name."""`

Use one-line `///` XML documentation comments for C# types and public APIs.

### Python

- Use `pathlib.Path` for all path manipulations. Do not use string concatenation or `os.path`.
- Use `Typer` for all CLI scripts.
- Scripts are internal; invoke them by their fully qualified module path from
  the Makefile and do not add `[project.scripts]` entrypoints.
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

- Application logic crates forbid `unsafe_code` and deny clippy warnings. The
  audited Windows bridge is the only exception because its exported C ABI uses
  unsafe attributes.
- Keep protocol and other pure logic in `keymap-core` where it can be tested
  without hardware. Shared I/O belongs in `keymap-overlay-runtime`; operating
  system integration belongs in the matching platform crate.
- Use `anyhow::Context` to attach the path or device to an error rather than
  logging and discarding it.

## Directory Structure

- `model/`: Python firmware metadata/conversion utilities, tests, and type stubs.
- `overlay/keymap-core/`: Platform-neutral Rust protocol and transition logic.
- `overlay/keymap-overlay-runtime/`: Library-only shared listener, model
  composition, transitions, command-line handling, and logging.
- `overlay/keymap-overlay-generator/`: Rust library that builds the display
  model natively, reading a connected device through its
  first-party Vial protocol client.
- `overlay/platforms/`: Platform-owned protocols, bridges, and native frontends.
- `firmware/`: First-party QMK integration, local keyboard configurations, and
  vendored firmware dependencies.
- `firmware/examples/`: One numbered directory
  per keyboard (configurable via `KEYBOARDS_DIR`).
- `installer/`: Verified release installers, release automation, and their tests.
- `tools/`: Repository-wide development checks.
- `docs/`: Design documentation and README images.
- `build/`: Generated JSON models. Not checked in.
- `firmware/vendor/vial-qmk/`: The Vial QMK firmware submodule.

## Important Files

- `Makefile`: The primary entry point for all automation.
- `docs/design.md`: Data flow, Raw HID protocol, and installation model.
- `lefthook.yml`: The git hooks and the make targets they run.
- `mise.toml`: Pinned tool versions and the `format`/`lint` tasks.
- `firmware/layer_notify.h`: Raw HID report construction and MO detection.
- `pyproject.toml`: Python dependencies and tool configurations.
- `installer/install.sh`, `installer/install.ps1`: verified release install, upgrade, rollback, and
  uninstall paths.
- `.github/workflows/release.yml`: cross-platform archives, checksums,
  attestations, and publishing.

# QMK Keymap Overlay

[![CI](https://github.com/sunaemon/keymap-overlay/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/sunaemon/keymap-overlay/actions/workflows/ci.yml?query=branch%3Amain)
[![License: MIT](https://img.shields.io/badge/License-MIT-green.svg)](LICENSE)
[![Example: GPL-2.0-or-later](https://img.shields.io/badge/Example-GPL--2.0--or--later-blue.svg)](example/LICENSE)
[![Platform: macOS | Linux | Windows](https://img.shields.io/badge/Platform-macOS%20%7C%20Linux%20%7C%20Windows-lightgrey.svg)](#platform-support)

This repository generates keyboard layer images from QMK keymaps and displays them in a native Rust overlay while layer modifier keys are held, on macOS, Linux and Windows.

![The overlay showing layer 1 of salicylic_acid3/insixty_en while its layer key is held](doc/images/overlay.png)

## Overview

See `doc/design.md` for the end-to-end data flow and design notes.

## Binary Releases

Tagged releases contain native binaries for macOS, Linux, and Windows. Download
the archive matching the operating system and CPU architecture, then run the
binary with the directory containing the layer PNGs:

```bash
keymap-overlay <asset-directory>
```

The release archive contains only the overlay binary. Generate the
keyboard-specific PNGs with `make install-assets` and keep them in the asset
directory; the Windows workflow below uses
`%USERPROFILE%/.config/keymap-overlay`. The source checkout is therefore only
needed to generate or update those assets, not to build the overlay binary.

This repo is set up for the `salicylic_acid3/insixty_en` and `doio/kb16/rev2` keyboards, but the workflow should work for any QMK-compatible keyboard with minor configuration changes. Keyboard configurations are stored in the `example/` directory (configurable via `KEYBOARDS_DIR` in the `Makefile`).

Each keyboard lives in a directory named after its `KEYBOARD_ID`. That ID is compiled into the firmware and travels in one byte of the Raw HID report, so it must be an **integer between 0 and 255** — `example/1`, `example/2`, and so on.

While holding a layer key, the firmware sends a Raw HID notification and the Rust app displays the matching layer image on screen.

## Platform Support

|                | macOS                       | Linux                                           | Windows                      |
| -------------- | --------------------------- | ----------------------------------------------- | ---------------------------- |
| Overlay window | eframe/egui                 | `zwlr_layer_shell_v1` surface, or an X11 window | eframe/egui                  |
| Autostart      | launchd agent               | systemd user unit                               | Task Scheduler task          |
| Raw HID access | Input Monitoring permission | `uaccess` udev rule (`make install-udev-rules`) | nothing to grant             |
| QMK toolchain  | Homebrew (`osx-cross`)      | distribution packages (pacman, apt, dnf)        | not supported — build in WSL |

Image generation is the same everywhere.

On Linux the overlay picks its window at startup:

- **Wayland with `zwlr_layer_shell_v1`** — COSMIC, sway, Hyprland, wayfire, KDE
  Plasma. This is the one to want: the compositor guarantees that the overlay
  stays above other windows, is never focused, and passes clicks through.
- **X11, or Wayland without layer-shell** — GNOME above all. The overlay falls
  back to an override-redirect X11 window, reached through XWayland in a Wayland
  session. The window manager does not manage it, so it is not focused and not
  restacked, but it is a fallback: it has none of the compositor's guarantees
  about other always-on-top windows.

Set `KEYMAP_OVERLAY_BACKEND` to `layer-shell` or `x11` to override the choice
(`auto` is the default). That is also how to try the fallback on a machine whose
compositor does support layer-shell.

Choose the setup that matches the system running the overlay:

- [macOS or Linux](#setup-on-macos-and-linux)
- [Windows](#setup-on-windows)

## Using as a Submodule

When using this project from a dotfiles repository, keep `keymap-overlay` as
an unmodified submodule and store keyboard-specific configuration beside it in
the parent repository. This lets the overlay stay easy to update while your
keymaps remain under your own version control.

```text
dotfiles/
├── keymap-overlay/           # this repository, as a submodule
└── keymap-overlay-keyboards/ # keyboard-specific configuration
    ├── 1/
    └── 2/
```

Seed the external configuration directory from the included examples:

```bash
cd ~/dotfiles
cp -R keymap-overlay/example keymap-overlay-keyboards
```

Run commands from the parent repository, passing the external directory with
`KEYBOARDS_DIR`. Use an absolute path because `make -C keymap-overlay` changes
the working directory:

```bash
cd ~/dotfiles
make -C keymap-overlay \
  KEYBOARDS_DIR="$PWD/keymap-overlay-keyboards" \
  compile KEYBOARD_ID=2
```

Use the same `KEYBOARDS_DIR` argument for `flash`, `flash-keymap`, and
`install-overlay`.

## Setup on macOS and Linux

These steps build the firmware, generate the assets, and install the overlay
on the same system.

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

3. Install the project dependencies and QMK toolchain:

   ```bash
   make setup
   ```

   On macOS, if you previously installed Homebrew core's `arm-none-eabi-gcc`,
   remove it first:

   ```bash
   brew uninstall arm-none-eabi-gcc
   ```

   The setup target installs QMK's supported `osx-cross` compiler instead.
   It configures the compiler path for project commands; no shell-profile changes are needed.

   On Linux it installs the ARM and AVR toolchains with `pacman`, `apt-get` or
   `dnf` — asking for `sudo` when it does — along with the libudev, Wayland
   client and libX11 libraries the overlay links against.

   It also installs the git hooks that format, lint, and test the project as
   you commit and push. See the Git Hooks section of `AGENTS.md` for what each
   hook runs and how to skip them.

4. Check setup (optional):

   ```bash
   make doctor
   ```

   This command is read-only: it reports missing dependencies but never installs them.

5. Flash firmware with Raw HID support to your keyboard:

   ```bash
   make flash KEYBOARD_ID=1
   ```

   `make flash` compiles first and then waits for the keyboard to appear in
   bootloader mode, so start the command and put the board into the bootloader
   while it waits.

   The recommended way is to keep a `QK_BOOT` key in the keymap and press it
   once `make flash` starts waiting. Both example keymaps already have one:

   | Keyboard             | `QK_BOOT` binding                              |
   | -------------------- | ---------------------------------------------- |
   | `1` (insixty_en)     | hold the `L1` key (right of `RSFT`), press `Q` |
   | `2` (doio/kb16/rev2) | hold the `MO(3)` key (bottom left), press `1`  |

   If the firmware on the board is broken or does not have `QK_BOOT`, use the
   hardware method instead:
   - **insixty_en**: hold the top-left key while plugging in the USB cable. The
     board mounts as a USB drive; the firmware is flashed by copying the `.uf2`
     onto it. Flash each half separately. See the
     [build guide](https://salicylic-acid3.hatenablog.com/entry/in60en-build-guide#Tips%E3%83%95%E3%82%A1%E3%83%BC%E3%83%A0%E3%82%A6%E3%82%A7%E3%82%A2%E3%82%92%E6%9B%B8%E3%81%8D%E6%8F%9B%E3%81%88%E3%82%8B).
   - **doio/kb16/rev2**: hold the `1!` key (matrix position 0,0) while plugging
     in, or briefly press the reset button on the back of the PCB. See the
     [QMK keyboard README](https://github.com/qmk/qmk_firmware/tree/master/keyboards/doio/kb16/rev2#bootloader).

   On Linux, a board with an `rp2040` bootloader appears as a `RPI-RP2` USB
   mass storage volume, and QMK only deploys to that volume once something
   else has mounted it. Nothing reliably does: desktops do not all auto-mount,
   and `udisks` refuses an SSH session or mounts under `/run/media/root`, where
   QMK cannot read it — so `qmk flash` sits at `Waiting for drive to deploy...`
   forever. `make flash` therefore mounts the volume itself, with `sudo`, at
   `/run/media/$USER/RPI-RP2`, and may prompt for a password once the board
   enters its bootloader. Set `SUDO=` to disable that (for a setup that
   auto-mounts already) or `UF2_VOLUME_LABEL=` for a differently labelled
   volume. Boards that flash over USB, such as `doio/kb16/rev2`, are
   unaffected.

   If the overlay service is already running, restart it after flashing so it
   reconnects to the keyboard's Raw HID interface:

   ```bash
   # macOS
   launchctl kickstart -k "gui/$(id -u)/com.sunaemon.keymap-overlay"
   # Linux
   systemctl --user restart keymap-overlay.service
   ```

6. On Linux, grant access to the keyboards' Raw HID nodes:

   ```bash
   make install-udev-rules
   ```

   This writes one `uaccess` rule per keyboard to
   `/etc/udev/rules.d/50-keymap-overlay.rules` (with `sudo`), so that whoever is
   logged in at the seat may read them. Without it the overlay finds the
   keyboards but cannot open them, and says so in its log. Replug a keyboard
   that was already connected. `make uninstall-udev-rules` removes the file.

   On macOS there is no equivalent step: grant the overlay **Input Monitoring**
   in System Settings when it first asks.

7. Install the native overlay as a login service:

   ```bash
   make install-overlay
   ```

   It starts automatically after login — as a launchd agent on macOS or a
   systemd user unit on Linux — and writes logs to
   `~/.local/var/log/keymap-overlay/overlay.log`. Logs rotate at 1 MiB,
   retaining the current log plus three previous files.
   For a foreground development session, use `make run-overlay` instead.
   To stop and remove it later, run `make uninstall-overlay`.

### Use VIAL

These commands are for users who have VIAL-enabled firmware on their keyboard.

1. Install the overlay using the current keymap in EEPROM instead of the compiled keymap:

   ```bash
   make install-overlay VIAL=true
   ```

2. Parse keymap.c and write the keymap to EEPROM:

   ```bash
   make flash-keymap
   ```

## Setup on Windows

Build and run the overlay from the **MSYS2 MSYS** shell. Do not use WSL for the
overlay: it cannot receive the keyboard's Raw HID events or display above native
Windows applications. The commands in this section use the Windows package
manager, `winget`, from PowerShell.

```powershell
winget install --id Git.Git -e
winget install --id MSYS2.MSYS2 -e
winget install --id jdx.mise -e
```

To build the native Windows executable from source, install the Microsoft C++
build tools too (this is a several-gigabyte download):

```powershell
winget install --id Microsoft.VisualStudio.2022.BuildTools -e --override "--wait --passive --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
```

Close and reopen Windows Terminal after the installs. Add an **MSYS2 MSYS**
profile with this command line. `-use-full-path` makes Windows-installed tools
such as mise and GitHub CLI available inside the shell.

```text
C:\msys64\msys2_shell.cmd -defterm -here -no-start -msys -use-full-path
```

Start a new MSYS2 MSYS tab. If the first update asks you to close the shell, do
so, reopen it, and run the update again. This also installs GNU Make, which is
not included with MSYS2 by default.

```bash
pacman -Syu
pacman -Syu
pacman -S --needed make
```

Verify that the shell inherited Git for Windows:

```bash
git --version
```

If `git` is not found, add Git for Windows to the MSYS2 shell's path, then
reload the shell configuration:

```bash
echo 'export PATH="/c/Program Files/Git/cmd:$PATH"' >> ~/.bashrc
source ~/.bashrc
```

Use the Windows home directory for the checkout so it is accessible from
Explorer, PowerShell, and MSYS2:

```bash
cd /c/Users/<your-Windows-user-name>
git -c core.autocrlf=false clone --recurse-submodules https://github.com/sunaemon/keymap-overlay.git
cd keymap-overlay
```

This checkout is shared by MSYS2 and WSL. It is safe to run `make setup` once
in each shell, as long as the commands do not run at the same time. WSL creates
keyboard images in `build/<keyboard-id>/`, while MSYS2 builds the Windows
executable in `target/`. If you also build or test Rust in WSL, keep its Cargo
output separate:

```bash
CARGO_TARGET_DIR=target-wsl make test-rust
```

### WSL asset setup

WSL generates the keyboard-specific PNG assets. If WSL is not installed, open
an administrator PowerShell and install Ubuntu:

```powershell
wsl --install -d Ubuntu
```

Restart if Windows asks, open the new Ubuntu terminal, and create the Linux
user it prompts for. In that terminal, install the basic build tools and mise:

```bash
sudo apt-get update
sudo apt-get install --yes build-essential curl git
curl https://mise.run/bash | sh
exec bash
```

Enter the checkout through WSL's mount of the Windows drive, then install its
Linux-side dependencies. This command may ask for your Linux password while it
installs the QMK toolchain. Give WSL its own Python virtual environment before
running setup, so a future Windows-side Python command cannot reuse it:

```bash
cd /mnt/c/Users/<your-Windows-user-name>/keymap-overlay
echo 'export UV_PROJECT_ENVIRONMENT=.venv-wsl' >> ~/.bashrc
export UV_PROJECT_ENVIRONMENT=.venv-wsl
make setup
```

Generate the assets directly into the Windows configuration directory:

```bash
make install-assets \
  KEYMAP_OVERLAY_DIR=/mnt/c/Users/<your-Windows-user-name>/.config/keymap-overlay
```

Run `make install-assets` again whenever the keymap changes. Then, from MSYS2
MSYS, build and install the native overlay:

```bash
make setup
make install-overlay
```

`install-overlay` deliberately does not generate images on Windows. It checks
for the PNGs generated by WSL, installs the `.exe`, and registers the login
task. Do not run `mise setup`: `setup` is a Makefile target, while mise is the
tool manager it invokes.

Building and flashing firmware is the one part that does not run from the
overlay's Windows shell. QMK's toolchain there is QMK MSYS, separate from
MSYS2. Run `make compile`, `make flash`, and `make flash-keymap` in QMK MSYS,
WSL, macOS, or Linux. To flash an already-built `.uf2`, put the keyboard in its
bootloader and copy the file onto the mounted `RPI-RP2` drive in Explorer.

## License

This project is licensed under multiple licenses. Keymap files in `example/` are under **GPL v2.0 or later**, while the tools and scripts are under the **MIT License**.

See [LICENSE](LICENSE) for details on file origins and full license texts.

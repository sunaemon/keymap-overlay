# QMK Keymap Overlay

[![CI](https://github.com/sunaemon/keymap-overlay/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/sunaemon/keymap-overlay/actions/workflows/ci.yml?query=branch%3Amain)
[![License: MIT](https://img.shields.io/badge/License-MIT-green.svg)](LICENSE)
[![Example: GPL-2.0-or-later](https://img.shields.io/badge/Example-GPL--2.0--or--later-blue.svg)](example/LICENSE)
[![Platform: macOS | Linux | Windows](https://img.shields.io/badge/Platform-macOS%20%7C%20Linux%20%7C%20Windows-lightgrey.svg)](#platform-support)

This project builds QMK firmware that reports momentary layer changes over Raw
HID, generates one PNG for each keymap layer, and displays the active layer in
a native overlay on macOS, Linux, and Windows.

![The overlay showing layer 1 of salicylic_acid3/insixty_en while its layer key is held](doc/images/overlay.png)

See [doc/design.md](doc/design.md) for the protocol, data flow, and windowing
design.

## How Installation Is Split

The normal workflow deliberately uses two distribution models:

- **Firmware and layer images come from the source checkout.** Keep the
  checkout because `make flash`, `make flash-keymap`, and
  `make install-assets` depend on the keyboard-specific files in `example/`
  and the vendored `qmk_firmware` submodule.
- **The overlay application comes from GitHub Releases.** `install.sh` or
  `install.ps` downloads the latest native binary and registers it to start at
  login. A normal installation does not compile the Rust overlay locally.

The release archive contains only the overlay executable. It does not contain
firmware or keyboard-specific PNGs.

This repository currently includes configurations for
`salicylic_acid3/insixty_en` and `doio/kb16/rev2`. Each directory under
`example/` is named with its `KEYBOARD_ID`. The ID is compiled into the
firmware, sent in the Raw HID report, and used as the PNG filename prefix, so it
must be an integer from 0 through 255.

## Platform Support

|                | macOS                       | Linux                                           | Windows                       |
| -------------- | --------------------------- | ----------------------------------------------- | ----------------------------- |
| Overlay window | eframe/egui                 | `zwlr_layer_shell_v1` surface, or an X11 window | eframe/egui                   |
| Autostart      | launchd agent               | systemd user unit                               | current-user Run registry key |
| Raw HID access | Input Monitoring permission | `uaccess` udev rule (`make install-udev-rules`) | nothing to grant              |
| Firmware tools | source checkout on macOS    | source checkout on Linux                        | source checkout in WSL        |
| Overlay binary | GitHub Release              | GitHub Release                                  | GitHub Release                |

On Linux, the overlay uses `zwlr_layer_shell_v1` on COSMIC, sway, Hyprland,
wayfire, and KDE Plasma. On GNOME, X11, and compositors without layer-shell, it
uses an override-redirect X11 window through XWayland or X11. Set
`KEYMAP_OVERLAY_BACKEND` to `auto`, `layer-shell`, or `x11` to override the
selection.

## Install on macOS or Linux

These steps build and flash the keyboard firmware from source, generate the
layer PNGs, and install the latest released overlay binary.

### 1. Clone the firmware and asset sources

```bash
git clone --recurse-submodules https://github.com/sunaemon/keymap-overlay.git
cd keymap-overlay
```

Install [mise](https://mise.jdx.dev/getting-started.html). With Homebrew:

```bash
brew install mise
```

Install the pinned tools and QMK toolchain:

```bash
make setup
```

On macOS, if Homebrew core's incompatible `arm-none-eabi-gcc` is already
installed, remove it before setup. The Makefile installs QMK's supported
`osx-cross` compiler instead.

```bash
brew uninstall arm-none-eabi-gcc
make setup
```

On Linux, `make setup` supports `pacman`, `apt-get`, and `dnf` and may ask for
the sudo password while installing system packages.

### 2. Build and flash the firmware

```bash
make flash KEYBOARD_ID=1
```

`make flash` compiles the firmware and waits for the keyboard to enter its
bootloader. Both example keymaps include `QK_BOOT`:

| Keyboard             | `QK_BOOT` binding                              |
| -------------------- | ---------------------------------------------- |
| `1` (insixty_en)     | hold the `L1` key (right of `RSFT`), press `Q` |
| `2` (doio/kb16/rev2) | hold the `MO(3)` key (bottom left), press `1`  |

If the installed firmware cannot enter the bootloader:

- **insixty_en:** hold the top-left key while connecting USB. Copy the built
  `.uf2` to the mounted `RPI-RP2` volume, flashing each half separately. See
  the [build guide](https://salicylic-acid3.hatenablog.com/entry/in60en-build-guide#Tips%E3%83%95%E3%82%A1%E3%83%BC%E3%83%A0%E3%82%A6%E3%82%A7%E3%82%A2%E3%82%92%E6%9B%B8%E3%81%8D%E6%8F%9B%E3%81%88%E3%82%8B).
- **doio/kb16/rev2:** hold the `1!` key while connecting USB, or press the
  reset button on the back. See the
  [QMK keyboard README](https://github.com/qmk/qmk_firmware/tree/master/keyboards/doio/kb16/rev2#bootloader).

On Linux, the Makefile mounts an rp2040 `RPI-RP2` volume at
`/run/media/$USER/RPI-RP2` before QMK deploys the `.uf2`. Set `SUDO=` if the
desktop already mounts it, or set `UF2_VOLUME_LABEL` for another volume label.

### 3. Generate the layer images

```bash
make install-assets
```

Run this again after changing the keymap. It installs PNGs such as `1_L1.png`
under `~/.config/keymap-overlay`.

On Linux, also grant the logged-in user access to the Raw HID interfaces and
reconnect any keyboard that was already plugged in:

```bash
make install-udev-rules
```

On macOS, grant the overlay Input Monitoring permission in System Settings
when prompted.

### 4. Install the latest overlay release

```bash
curl -fsSLO https://raw.githubusercontent.com/sunaemon/keymap-overlay/main/install.sh
sh install.sh
```

The installer selects the release for the current OS and architecture,
installs the executable, registers the launchd agent or systemd user unit, and
starts it. It prints every installed path when complete. Logs are written to
`~/.local/var/log/keymap-overlay/overlay.log` and rotate at 1 MiB, retaining
the current file and three previous files.

Run `sh install.sh` again to upgrade to the latest release.

## Install on Windows

The normal Windows workflow uses two environments:

- **WSL Ubuntu** holds the source checkout, builds and flashes QMK firmware,
  and generates PNGs.
- **PowerShell** installs and runs the released native Windows overlay.

MSYS2 and Visual Studio Build Tools are not required unless developing the
Windows overlay itself; that setup is documented under
[Getting Started for Development](#getting-started-for-development).

### 1. Set up the firmware environment in WSL

Open an administrator PowerShell:

```powershell
wsl --install -d Ubuntu
winget install --interactive --exact dorssel.usbipd-win
wsl --update
```

Restart Windows if requested, open Ubuntu, create its Linux user, and install
the base tools and mise:

```bash
sudo apt-get update
sudo apt-get install --yes build-essential curl git usbutils
curl https://mise.run/bash | sh
exec bash
```

Clone the source inside WSL's Linux filesystem and install the QMK and image
generation tools:

```bash
git clone --recurse-submodules https://github.com/sunaemon/keymap-overlay.git
cd keymap-overlay
make setup
```

Keeping this checkout under the WSL home directory avoids Windows filesystem
performance and Python virtual-environment compatibility problems.

### 2. Build and flash the firmware from WSL

Keep the keyboard attached to Windows during normal use. Put it into its
bootloader, then identify the newly enumerated bootloader in an administrator
PowerShell:

```powershell
usbipd list
usbipd bind --busid <BUSID>
```

Sharing persists for that bootloader device. With an Ubuntu terminal already
open, attach it from a normal PowerShell:

```powershell
usbipd attach --wsl --busid <BUSID>
```

Then flash from the WSL checkout:

```bash
lsusb
make flash KEYBOARD_ID=1
```

The bootloader detaches when the keyboard resets or is unplugged. To detach it
manually, run `usbipd detach --busid <BUSID>` in PowerShell.

If USBPcap is installed, `usbipd` may require
`usbipd bind --force --busid <BUSID>`. Apply that only to the bootloader entry,
not the normal keyboard. For an rp2040 bootloader, building in WSL and copying
the resulting `.uf2` to `RPI-RP2` in Explorer is also a simple fallback.

### 3. Generate images into the Windows profile

From the WSL checkout:

```bash
WINDOWS_HOME="$(wslpath "$(cmd.exe /C echo %USERPROFILE% | tr -d '\r')")"
make install-assets \
  KEYMAP_OVERLAY_DIR="$WINDOWS_HOME/.config/keymap-overlay"
```

Run this again after changing the keymap.

### 4. Install the latest Windows overlay release

Download [install.ps](install.ps), then run it from PowerShell in the download
directory:

```powershell
powershell.exe -ExecutionPolicy Bypass -File .\install.ps
```

The installer downloads the Windows x86_64 release, installs it under
`%USERPROFILE%\.config\keymap-overlay`, registers the current user's
`KeymapOverlay` Run value, starts the overlay, and prints every installed
location. No administrator PowerShell is needed for this step.

Run the same command again to upgrade to the latest release.

## Updating a Keymap

The source checkout remains the authority for firmware and images:

```bash
git pull --recurse-submodules
git submodule update --init --recursive
make flash KEYBOARD_ID=<keyboard-id>
make install-assets
```

On Windows, use the WSL `KEYMAP_OVERLAY_DIR` argument shown above. Restart the
overlay after flashing so it reconnects immediately:

```bash
# macOS
launchctl kickstart -k "gui/$(id -u)/com.sunaemon.keymap-overlay"

# Linux
systemctl --user restart keymap-overlay.service
```

On Windows PowerShell:

```powershell
Get-Process keymap-overlay -ErrorAction SilentlyContinue | Stop-Process
$overlay = "$env:USERPROFILE\.config\keymap-overlay"
Start-Process "$overlay\keymap-overlay.exe" -ArgumentList "`"$overlay`""
```

## VIAL Keymaps

For a keyboard running VIAL-enabled firmware, generate images from the keymap
currently stored in EEPROM:

```bash
make install-assets VIAL=true
```

To parse `keymap.c` and write the result to EEPROM without rebuilding the
firmware:

```bash
make flash-keymap
```

`flash-keymap` preserves `KC_TRNS` on the device-writing path so transparent
keys continue to inherit lower layers.

## Using as a Submodule

When this project belongs to a dotfiles repository, keep it as an unmodified
submodule and store keyboard-specific configuration beside it:

```text
dotfiles/
├── keymap-overlay/           # this repository, as a submodule
└── keymap-overlay-keyboards/ # private keyboard configuration
    ├── 1/
    └── 2/
```

Seed the external configuration from the included examples:

```bash
cd ~/dotfiles
cp -R keymap-overlay/example keymap-overlay-keyboards
```

Run source-based firmware and image commands with an absolute
`KEYBOARDS_DIR`, because `make -C` changes the working directory:

```bash
cd ~/dotfiles
make -C keymap-overlay \
  KEYBOARDS_DIR="$PWD/keymap-overlay-keyboards" \
  flash KEYBOARD_ID=2

make -C keymap-overlay \
  KEYBOARDS_DIR="$PWD/keymap-overlay-keyboards" \
  install-assets
```

The binary installer is unchanged; it reads the generated PNGs from the
platform configuration directory.

## Getting Started for Development

This section is for changing the Rust overlay, Python generators, Makefile, or
installation scripts. Normal users can stop at the platform installation
sections above.

### macOS and Linux development

Clone the repository, initialize every submodule, and install the complete
toolchain:

```bash
git clone --recurse-submodules https://github.com/sunaemon/keymap-overlay.git
cd keymap-overlay
make setup
```

Build and test the overlay from source:

```bash
make test
make test-rust
make build-overlay
```

To exercise the source-built installation path rather than a Release binary:

```bash
make install-overlay
```

For a foreground UI session, use `make run-overlay`.

### Windows overlay development with MSYS2

The released Windows overlay is native, so develop it from **MSYS2 MSYS**, not
WSL. Install Git, MSYS2, mise, and the Microsoft C++ build tools from
PowerShell:

```powershell
winget install --id Git.Git -e
winget install --id MSYS2.MSYS2 -e
winget install --id jdx.mise -e
winget install --id Microsoft.VisualStudio.2022.BuildTools -e --override "--wait --passive --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
```

Add a Windows Terminal profile with this command line:

```text
C:\msys64\msys2_shell.cmd -defterm -here -no-start -msys -use-full-path
```

Open that profile, complete the MSYS2 update, and install GNU Make:

```bash
pacman -Syu
pacman -Syu
pacman -S --needed make
```

`-use-full-path` normally exposes Git for Windows. If `git` is still missing:

```bash
echo 'export PATH="/c/Program Files/Git/cmd:$PATH"' >> ~/.bashrc
source ~/.bashrc
```

Clone into the Windows profile and set up the native development tools:

```bash
WINDOWS_HOME="$(cygpath -u "$USERPROFILE")"
cd "$WINDOWS_HOME"
git -c core.autocrlf=false clone --recurse-submodules https://github.com/sunaemon/keymap-overlay.git
cd keymap-overlay
make setup
```

Run the Windows test and build paths:

```bash
make test
make test-rust
make build-overlay
make install-overlay
```

`make install-overlay` expects layer PNGs already generated into the Windows
profile by WSL. Firmware targets intentionally stop in MSYS2; run `compile`,
`flash`, `flash-keymap`, and `install-assets` from the WSL source checkout.

The Windows CI job compiles `ui/windows.rs` but does not run clippy. After
changing that file, also run:

```bash
cargo clippy --target x86_64-pc-windows-msvc -p keymap-overlay -- -D warnings
```

### Verification commands

```bash
make format
make lint
make test
make test-rust
make build-overlay
make audit
```

On Windows, verify that the overlay never takes focus: type into an editor,
hold a layer key while continuing to type, and confirm every keystroke remains
in the editor. Repeat the test because the Windows show/focus failure mode can
appear only after the second display.

## License

Keymap files in `example/` are licensed under GPL-2.0-or-later. The tools and
scripts are licensed under the MIT License. See [LICENSE](LICENSE) and
[example/LICENSE](example/LICENSE).

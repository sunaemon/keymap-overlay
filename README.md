# QMK Keymap Overlay

[![CI](https://github.com/sunaemon/keymap-overlay/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/sunaemon/keymap-overlay/actions/workflows/ci.yml?query=branch%3Amain)
[![Codecov](https://codecov.io/gh/sunaemon/keymap-overlay/branch/main/graph/badge.svg)](https://app.codecov.io/gh/sunaemon/keymap-overlay)
[![Tools: MIT](https://img.shields.io/badge/Tools-MIT-green.svg)](LICENSE.md#tools-and-application-mit)
[![Firmware: GPL-2.0-or-later](https://img.shields.io/badge/Firmware-GPL--2.0--or--later-blue.svg)](LICENSE.md#firmware-and-qmk-keymap-files-gpl-20-or-later)
[![Platform: macOS | Linux | Windows](https://img.shields.io/badge/Platform-macOS%20%7C%20Linux%20%7C%20Windows-lightgrey.svg)](#platform-support)
[![Status: Beta](https://img.shields.io/badge/Status-Beta-orange.svg)](#project-status)

QMK Keymap Overlay shows the active momentary keyboard layer while its
`MO(...)` key is held. QMK firmware reports layer changes over Raw HID, and a
native overlay draws the matching keys, encoders, and labels on macOS, Linux,
or Windows. Release the layer key to hide the overlay or return to another
held layer. The overlay stays above other windows without taking focus or
intercepting clicks.

![The overlay showing layer 1 of salicylic_acid3/insixty_en while its layer key is held](docs/images/overlay.png)

## Project Status

> [!NOTE]
> This is beta software tested on the maintainer's systems. Compatibility and
> installation behavior may change before 1.0. See the
> [compatibility policy](docs/compatibility.md) and report problems through
> [GitHub Issues](https://github.com/sunaemon/keymap-overlay/issues).

## Choose Your Path

- [Install on macOS or Linux](#install-on-macos-or-linux)
  - [Finish setup on macOS](#finish-setup-on-macos)
  - [Finish setup on Linux](#finish-setup-on-linux)
  - [Enable GNOME](#gnome)
  - [Enable KDE Plasma, Cinnamon, or another desktop](#kde-plasma-cinnamon-and-other-desktops)
- [Install on Windows](#install-on-windows)
- [Update a keymap or the overlay](#everyday-operations)
- [Use a Vial keymap](#vial-keymaps)
- [Add your own keyboard](docs/custom-keyboards.md)
- [Develop keymap-overlay](#development)

## How It Works

Installation has two parts:

1. Build and flash QMK firmware that embeds the keyboard metadata and reports
   momentary layer changes over Raw HID.
2. Install the released native overlay and its login service. At startup the
   overlay reads each connected keyboard's Vial definition and keymap directly
   into memory.

Firmware comes from this source checkout. The native overlay comes from GitHub
Releases, so a normal installation does not compile Rust locally. Release
archives contain the executable, MIT license, and third-party notices. Keyboard
definitions and generated layer models are not installed on the host.

If your keyboard already runs compatible firmware, skip directly to the
released overlay installer for [macOS/Linux](#3-install-the-released-overlay)
or [Windows](#3-install-the-released-windows-overlay). On Linux, first complete
the source setup to install runtime dependencies and provide the
`make install-udev-rules` command used below; you can skip flashing.

The overlay reads connected keyboards at startup and automatically loads a
compatible keyboard connected later. Restart it after editing a keymap in
Vial. Only momentary `MO(...)` layers trigger the overlay; toggle and layer-tap
keys do not.

See [docs/design.md](docs/design.md) for the Raw HID protocol, data flow, layer
composition rules, and native window design.

## Platform Support

|                | macOS                       | Linux                                   | Windows                       |
| -------------- | --------------------------- | --------------------------------------- | ----------------------------- |
| Renderer       | AppKit                      | GNOME Shell or Qt Quick                 | Rust / windows-rs + Win32     |
| Autostart      | launchd                     | systemd user services + GNOME extension | current-user Run registry key |
| Raw HID access | Input Monitoring permission | `uaccess` udev rule                     | no additional permission      |
| Firmware tools | source checkout             | source checkout                         | source checkout in WSL        |
| Overlay binary | GitHub Release              | GitHub Release                          | GitHub Release                |

GNOME 45 or newer uses the included Shell extension on Wayland or X11. Other
desktops use the Qt renderer on both display protocols. On Wayland,
LayerShellQt provides the required overlay semantics; on X11, the renderer uses
a native Qt/XCB window. The maintainer has tested GNOME/Wayland, KDE
Plasma/Wayland, Sway/Wayland, and Cinnamon/X11. Other combinations are
supported by their respective renderer but are not part of the regularly
tested matrix. Cinnamon does not load the GNOME extension.

### Bundled keyboards

| ID  | QMK keyboard                 | `QK_BOOT` binding                           |
| --- | ---------------------------- | ------------------------------------------- |
| `1` | `salicylic_acid3/insixty_en` | hold `L1` (right of `RSFT`), then press `Q` |
| `2` | `doio/kb16/rev2`             | hold `MO(3)` (bottom left), then press `1`  |

Each directory under `firmware/examples/` is named with its `KEYBOARD_ID`. The ID is
compiled into firmware and used to match Raw HID events to in-memory models; it must be
an integer from 0 through 255. Other keyboards require a small configuration;
see [Custom Keyboard Configuration](docs/custom-keyboards.md).

If the installed firmware cannot enter the bootloader:

- **insixty_en:** hold the top-left key while connecting USB. Copy the built
  `.uf2` to the mounted `RPI-RP2` volume, flashing each half separately. See
  the [build guide](https://salicylic-acid3.hatenablog.com/entry/in60en-build-guide#Tips%E3%83%95%E3%82%A1%E3%83%BC%E3%83%A0%E3%82%A6%E3%82%A7%E3%82%A2%E3%82%92%E6%9B%B8%E3%81%8D%E6%8F%9B%E3%81%88%E3%82%8B).
- **doio/kb16/rev2:** hold `1!` while connecting USB, or press the reset button
  on the back. See the
  [QMK keyboard README](https://github.com/qmk/qmk_firmware/tree/master/keyboards/doio/kb16/rev2#bootloader).

## Install on macOS or Linux

### 1. Prepare the source checkout

On macOS, `make setup` requires Homebrew (`brew`) to install the QMK toolchain.

```bash
git clone https://github.com/sunaemon/keymap-overlay.git
cd keymap-overlay
curl https://mise.run | sh
export PATH="$HOME/.local/bin:$PATH"
```

See [Installing mise](https://mise.jdx.dev/installing-mise.html) for package
manager alternatives and persistent shell setup. Install the pinned tools
and QMK toolchain:

```bash
make setup
```

`make setup` initializes Vial QMK shallowly and downloads only ChibiOS, LUFA,
and the Pico SDK, which are the nested firmware dependencies used by the
tracked STM32F103 and RP2040 keyboards. A keyboard with an unknown processor
produces a warning and falls back to initializing every nested submodule.
The firmware submodule opts out of Git's automatic updates, so cloning with
`--recurse-submodules` is also safe: Git skips firmware until `make setup`
selects its dependencies.

On Linux, `make setup` supports `pacman`, `apt-get`, and `dnf` and may request
`sudo` while installing system packages. Ubuntu 24.04 does not provide
`qml6-module-org-kde-layershell`; KDE users need a Qt 6 LayerShellQt package
from a newer distribution or backport. GNOME does not need LayerShellQt.

### 2. Build and flash firmware

Choose an ID from [Bundled keyboards](#bundled-keyboards):

```bash
make flash KEYBOARD_ID=1
```

`make flash` waits for the keyboard bootloader. On Linux, it reuses a writable
RP2040 bootloader volume or mounts it at `/run/media/$USER/RPI-RP2` with `sudo`.
It also cleans up a remaining mount at that path after flashing. Set
`UF2_VOLUME_LABEL` if your bootloader uses another volume label.

### 3. Install the released overlay

```bash
curl -fsSL \
  https://github.com/sunaemon/keymap-overlay/releases/latest/download/install.sh \
  -o install.sh
less install.sh
sh install.sh
```

Review the script before leaving `less` with `q`. It selects the release for
the current system, verifies its SHA-256 checksum, installs the executable,
registers login services, and starts them. Keep the keyboard
connected for the first start. When authenticated GitHub CLI is available, it
also verifies the artifact attestation.

On macOS and Linux the executable is installed to `~/.local/bin/keymap-overlay`.
Linux also installs the Qt renderer beside it. The login service uses an
absolute path, so `~/.local/bin` does not need to be on your `PATH`.
You can read the bundled license terms with:

```bash
~/.local/bin/keymap-overlay --license                 # this project's terms
~/.local/bin/keymap-overlay --third-party-licenses    # third-party notices
```

### Finish setup on macOS

Grant the overlay Input Monitoring permission in System Settings when
prompted. The overlay then appears whenever a QMK `MO(...)` key is held.
If it does not appear after granting permission, [restart the overlay](#restart-the-overlay).

### Finish setup on Linux

Grant Raw HID access, then reconnect keyboards that are already plugged in:

```bash
make install-udev-rules
```

[Restart the overlay](#restart-the-overlay) after reconnecting so it can read
the keyboard's model.

The installer provides both Linux renderers. Complete only the subsection for
your desktop.

#### GNOME

Log out and back in once so GNOME discovers the extension, then run:

```bash
gnome-extensions enable keymap-overlay@sunaemon
systemctl --user restart keymap-overlay.service
```

#### KDE Plasma, Cinnamon, and other desktops

The installer normally enables the Qt renderer automatically. To enable it
manually and start its shared HID daemon:

```bash
systemctl --user enable --now keymap-overlay.service keymap-overlay-qt.service
```

Cinnamon uses this Qt renderer rather than the GNOME Shell extension. It works
in a Cinnamon/X11 session through Qt's XCB backend; LayerShellQt is needed only
for supported Wayland sessions. If the service was already running under a
different desktop session, restart both processes after logging in so they
inherit Cinnamon's current `DISPLAY` and D-Bus environment:

```bash
systemctl --user restart keymap-overlay.service keymap-overlay-qt.service
```

Sway's default configuration imports its Wayland environment into the systemd
user manager but does not start `graphical-session.target`. The enabled overlay
services therefore remain inactive. Start them explicitly after entering a
Sway session:

```bash
systemctl --user start keymap-overlay.service keymap-overlay-qt.service
```

If the daemon was still acquiring its D-Bus name when the renderer started,
restart the renderer once:

```bash
systemctl --user restart keymap-overlay-qt.service
```

## Install on Windows

Building firmware and running the overlay on Windows use two environments:

- **WSL `keymap-firmware`** builds and flashes QMK firmware.
- **PowerShell** installs the released native overlay, which reads layer models
  directly from the connected keyboard.

Visual Studio Build Tools are needed only for source development. Windows App
Runtime is needed only for the experimental WinUI prototype.

Install Windows Terminal first, in an administrator PowerShell:

```powershell
winget install --interactive --exact --source winget Microsoft.WindowsTerminal
```

### 1. Prepare WSL

In an administrator PowerShell:

```powershell
winget install --interactive --exact --source winget dorssel.usbipd-win
wsl.exe --install --no-distribution
```

Restart if requested. In a new administrator PowerShell:

```powershell
wsl --update
wsl --install -d Ubuntu --name keymap-firmware
```

Restart if requested. In the new Ubuntu environment:

```bash
sudo apt-get update
sudo apt-get install --yes build-essential curl git usbutils
curl https://mise.run/bash | sh
exec bash
git clone https://github.com/sunaemon/keymap-overlay.git
cd keymap-overlay
make setup
sudo groupadd --force qmk
sudo usermod --append --groups qmk "$USER"
sed 's/TAG+="uaccess"/GROUP="qmk", MODE="0660"/' firmware/vendor/vial-qmk/util/udev/50-qmk.rules |
  sudo tee /etc/udev/rules.d/50-qmk-wsl.rules >/dev/null
sudo udevadm control --reload
newgrp qmk
```

Keep the checkout in the WSL home directory. The dedicated `qmk` group replaces
QMK's normal desktop-seat `uaccess` rule inside WSL. If no udev control socket
exists, run `sudo service udev restart` instead.

### 2. Attach and flash the bootloader

Enter the keyboard bootloader. In an administrator PowerShell, identify the
new bootloader entry—not the normal keyboard—and attach it to WSL:

```powershell
usbipd list
usbipd bind --busid <BUSID>
usbipd attach --wsl --busid <BUSID>
```

Keep a WSL terminal open, then flash from the checkout:

```bash
lsusb
make flash KEYBOARD_ID=1
```

After the bootloader resets, return it to Windows:

```powershell
usbipd detach --busid <BUSID>
```

Sharing persists, but attachment must be repeated after a disconnect. If
USBPcap is installed, `usbipd bind` may require `--force`; use it only for the
bootloader entry. For rp2040, copying the built `.uf2` to `RPI-RP2` in Explorer
is a simpler fallback.

### 3. Install the released Windows overlay

In a non-administrator PowerShell:

```powershell
Invoke-WebRequest `
  -Uri 'https://github.com/sunaemon/keymap-overlay/releases/latest/download/install.ps1' `
  -OutFile install.ps1 `
  -ErrorAction Stop
Get-Content -LiteralPath install.ps1
powershell.exe -ExecutionPolicy Bypass -File install.ps1
```

Review the script before running the final command. It verifies the release,
installs the executable under `%LOCALAPPDATA%\Programs\keymap-overlay`, registers
the current user's `KeymapOverlay` Run value, and starts the overlay. Keep the
keyboard connected while the overlay runs.

## Everyday Operations

### Edit a keymap in Vial

Edit the connected keyboard in the Vial app, then choose **Reload Keyboards**
from the tray menu to display the changes. Vial edits persist across keyboard
restarts; no firmware build is needed.

### Update firmware or the source keymap

To update the checkout and its firmware dependencies:

```bash
git pull
make setup-firmware
```

Edit the keyboard's `keymap.c` as needed, then build and flash it:

```bash
make flash KEYBOARD_ID=<keyboard-id>
```

`make flash` gives the firmware a fresh EEPROM epoch. On first boot the new
firmware resets Vial's EEPROM-backed configuration and initializes the dynamic
keymap from `keymap.c`; live Vial edits are therefore replaced by the source
keymap. On Windows, build and flash in WSL (or copy the built `.uf2` from WSL
to the bootloader volume in Explorer). Restart the overlay afterward.

### Configure the overlay

Use the keyboard icon in the macOS menu bar, Windows notification area, KDE or
other desktop system tray, or GNOME top panel. Its menu can control whether the
overlay launches at login, place it at the top, centre, or bottom of the active
display, choose 50–100% opacity, choose 75–150% scale, reload connected
keyboards, and quit. The installed version is shown in the same menu.

Presentation choices persist per user. **Launch at Login** updates the same
launchd, systemd, or Windows Run entry created by the installer without
stopping the current session. **Reload Keyboards** rereads connected devices in
the running process so current Vial EEPROM state appears immediately after a
live edit.

### Restart the overlay

Keep the keyboard connected while restarting. The overlay reads its current
keymap into memory at startup.

```bash
# macOS
launchctl kickstart -k "gui/$(id -u)/com.sunaemon.keymap-overlay"

# Linux daemon
systemctl --user restart keymap-overlay.service

# Linux Qt renderer, when used
systemctl --user restart keymap-overlay-qt.service
```

On Windows PowerShell:

```powershell
Get-Process keymap-overlay -ErrorAction SilentlyContinue | Stop-Process
$exe = "$env:LOCALAPPDATA\Programs\keymap-overlay\keymap-overlay.exe"
Start-Process $exe
```

### Upgrade the released overlay

```bash
# macOS or Linux
sh ~/.config/keymap-overlay/install.sh
```

```powershell
# Windows
powershell.exe -ExecutionPolicy Bypass -File "$env:LOCALAPPDATA\keymap-overlay\install.ps1"
```

### Logs and quick checks

Where the log goes depends on what supervises the overlay:

| System  | Log                                                |
| ------- | -------------------------------------------------- |
| Linux   | the journal, `journalctl --user -u keymap-overlay` |
| macOS   | `~/.local/var/log/keymap-overlay/overlay.log`      |
| Windows | `%LOCALAPPDATA%\keymap-overlay\logs\overlay.log`   |

A log the overlay writes itself rotates at 1 MiB and retains the current file
plus three previous files; journald applies its own retention instead.

On Linux:

```bash
systemctl --user status keymap-overlay.service
systemctl --user status keymap-overlay-qt.service
journalctl --user -u keymap-overlay.service -f
```

On macOS:

```bash
tail -f ~/.local/var/log/keymap-overlay/overlay.log
```

### Uninstall

```bash
# macOS or Linux
sh ~/.config/keymap-overlay/install.sh --uninstall
```

```powershell
# Windows
powershell.exe -ExecutionPolicy Bypass -File "$env:LOCALAPPDATA\keymap-overlay\install.ps1" -Uninstall
```

Uninstalling removes the executables, license notices, login entry, and any
legacy model cache. Rotated logs are retained.

## Vial Keymaps

The installed overlay reads the keymap currently stored in the connected
keyboard's Vial EEPROM at startup, including edits made in the Vial app.
It reads the device without writing to it or polling for keymap changes.
Choose **Reload Keyboards** from the tray menu after making live edits.

To make `keymap.c` the keyboard's live keymap, use `make flash`; the firmware's
fresh EEPROM epoch resets Vial state and initializes it from the compiled
source. `KC_TRNS` remains intact in firmware, so transparent keys continue to
inherit lower layers.

## Custom Keyboard Configuration

To add a keyboard, place encoder geometry, custom labels, and keymap source in
an external configuration directory or a fork. The complete workflow is in
[docs/custom-keyboards.md](docs/custom-keyboards.md).

## Development

Use this section to build from source or change the Rust overlay, Python
generators, Makefile, or installers.

### macOS and Linux

After completing the source setup from the installation section:

```bash
make test
make test-rust
make build-overlay
make install-overlay
```

`make install-overlay` exercises the source-built installation path. Use
`make run-overlay` for a foreground UI session.

On macOS, `make test-release-acceptance-macos` tests installer upgrades and
rollback, then drives simulated layer events through the native AppKit overlay
and verifies two show cycles with the hidden transition between them.

On Linux, `make test-release-acceptance-linux` is the equivalent release
go/no-go check. It tests installer upgrades and rollback, then runs two E2E
halves around the project-owned D-Bus contract. The HID-to-D-Bus half creates a
virtual device through `/dev/uhid`, sends Raw HID layer reports through the
kernel `hidraw` path, and asserts the daemon's public state. The
D-Bus-to-renderer half exercises the daemon and Qt renderer together, including
a golden-image comparison of the rendered Qt Quick overlay. The HID half
requires the `uhid` kernel module.

To exercise the native overlay with a built-in test model and simulated
momentary layer events:

```bash
make run-overlay SIMULATE=1:2
```

This shows keyboard 1 layer 2 for two seconds, hides it for one second, and
repeats until interrupted. Simulation replaces Raw HID input and supplies an
in-memory test model, so it works without a supported keyboard attached.

To export a connected keyboard's live Vial keymap as a display model for
inspection, clear generated files first so Make performs a fresh device read:

```bash
make clean
make draw-layers KEYBOARD_ID=1
```

This development command writes JSON under `build/<keyboard-id>/assets/`; the installed
overlay keeps its models in memory and does not read these files.

### Windows native overlay development

Develop the native Windows overlay in PowerShell, not WSL. The frontend is a
single Rust executable built directly with Cargo. In PowerShell:

```powershell
winget install --id jdx.mise -e --source winget
winget install --id GitHub.cli -e --source winget
winget install --id Microsoft.VisualStudio.2022.BuildTools -e --source winget --override "--wait --passive --add Microsoft.VisualStudio.Workload.VCTools --add Microsoft.VisualStudio.Component.VC.Tools.ARM64 --includeRecommended"
```

The production frontend uses only the Windows APIs exposed by `windows-rs`;
Windows App Runtime is optional and needed only for the experimental WinUI build.

Open a new terminal after WinGet finishes, then authenticate GitHub CLI for
pull requests, checks, releases, and artifact attestation verification:

```powershell
gh auth login
gh auth status
```

WinGet normally exposes `mise.exe` through
`%LOCALAPPDATA%\Microsoft\WinGet\Links`. If `mise` is not found after opening a
new terminal, add that directory to the user `PATH` from PowerShell, then close
and reopen Windows Terminal (and any editor or Codex process that launches
builds):

```powershell
$miseBin = "$env:LOCALAPPDATA\Microsoft\WinGet\Links"
$userPath = [Environment]::GetEnvironmentVariable("Path", "User")
if (($userPath -split ";") -notcontains $miseBin) {
    $updatedPath = if ([string]::IsNullOrWhiteSpace($userPath)) {
        $miseBin
    } else {
        $userPath.TrimEnd(";") + ";" + $miseBin
    }
    [Environment]::SetEnvironmentVariable(
        "Path",
        $updatedPath,
        "User"
    )
}
```

Open the architecture-matching Visual Studio developer command prompt (`ARM64
Native Tools Command Prompt for VS 2022` on Windows on Arm, or `x64 Native
Tools Command Prompt for VS 2022` on x64). Start PowerShell from that prompt
so it inherits the compiler environment:

```cmd
powershell.exe -NoProfile
```

Then run these commands in that PowerShell session:

```powershell
git clone https://github.com/sunaemon/keymap-overlay.git
Set-Location keymap-overlay
.\tools\windows.ps1 -Task setup
.\tools\windows.ps1 -Task build
.\tools\windows.ps1 -Task run -Simulate 1:2
.\tools\windows.ps1 -Task test-rust
```

The native build follows the host architecture. Confirm it before building:

```powershell
mise exec -- rustc -vV | Select-String '^host:'
```

It must be `x86_64-pc-windows-msvc` on x64 or `aarch64-pc-windows-msvc` on
Windows on Arm. The Visual Studio install command above includes both x64 and
ARM64 C++ tools so either build has the MSVC linker and Windows SDK it needs.

The release installer remains a separate PowerShell entry point because it
downloads and verifies a published archive. The development script invokes
Cargo, uv, Pester, and mise directly without a Unix compatibility layer. At
startup, the overlay reads every connected self-describing Vial keyboard into
memory. Device-arrival notifications trigger the same in-memory read for a
compatible keyboard connected later, without restarting the overlay.

For the complete Windows verification and source-install workflows:

```powershell
.\tools\windows.ps1 -Task format
.\tools\windows.ps1 -Task lint
.\tools\windows.ps1 -Task test
.\tools\windows.ps1 -Task test-rust
.\tools\windows.ps1 -Task test-release-acceptance
.\tools\windows.ps1 -Task install
```

Use `-Task uninstall` to remove the source installation while retaining logs.
QMK firmware builds remain unsupported in native Windows PowerShell; use WSL,
macOS, or Linux for firmware compilation and flashing.

### Verification

```bash
make format
make lint
make test
make test-rust
make build-overlay
make test-release-acceptance-macos # macOS only: installer rollback + AppKit E2E
make test-release-acceptance-linux # Linux only: installer rollback + both E2E halves
make audit
make test-installer-sh
```

Windows release acceptance runs the `install.ps1` Pester suite and the native E2E
test, which checks visible, hidden, and visible-again presentation states using
simulated layer events. Verify manually that the overlay never takes focus:
type into an editor while repeatedly holding a layer key and confirm every
keystroke remains in the editor.

See [CONTRIBUTING.md](CONTRIBUTING.md) for contribution expectations and
[docs/releasing.md](docs/releasing.md) for the release checklist.

## License

Firmware under `firmware/` and keyboard files under `firmware/examples/` are
GPL-2.0-or-later. The tools and application are MIT licensed. See
[LICENSE.md](LICENSE.md), [firmware/examples/LICENSE](firmware/examples/LICENSE), and the generated
[overlay dependency notices](THIRD-PARTY-LICENSES.html).

The installed overlay embeds its MIT license and dependency notices on every
platform, so a binary copied away from its install directory can still state
its terms: run `keymap-overlay --license` or
`keymap-overlay --third-party-licenses`. Release archives also include the
notice file for downstream packaging.

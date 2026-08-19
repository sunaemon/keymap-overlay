# Compatibility Policy

keymap-overlay is currently beta software, tested on the maintainer's systems.
Before 1.0, releases may change installation paths, configuration, generated
assets, or the Raw HID protocol when the change is documented in the release
notes. Backward compatibility is a goal, not yet a guarantee.

## Tested Platforms

Release CI builds and tests these targets:

| Platform | Release architecture    | Runtime expectation                                                |
| -------- | ----------------------- | ------------------------------------------------------------------ |
| macOS    | Apple silicon (`arm64`) | Current GitHub-hosted macOS runner and Input Monitoring permission |
| Linux    | `x86_64`                | GNOME 45+ or Qt 6/LayerShellQt, with systemd user services         |
| Windows  | `x86_64`                | Native Windows overlay installed from PowerShell                   |

Firmware compilation and flashing are supported on macOS and Linux. On a
Windows host, use WSL for firmware and layer-image work; the overlay itself is
native Windows software. Other architectures, init systems, and Windows on Arm
are not currently release targets.

## Installed Layout

Each system uses its own convention for per-user files:

|                       | macOS and Linux                                               | Windows                                  |
| --------------------- | ------------------------------------------------------------- | ---------------------------------------- |
| Executables           | `~/.local/bin`                                                | `%LOCALAPPDATA%\Programs\keymap-overlay` |
| Layer models          | `~/.cache/keymap-overlay`                                     | `%LOCALAPPDATA%\keymap-overlay`          |
| Installer bookkeeping | `~/.config/keymap-overlay`                                    | `%LOCALAPPDATA%\keymap-overlay`          |
| Logs                  | journald on Linux; `~/.local/var/log/keymap-overlay` on macOS | `%LOCALAPPDATA%\keymap-overlay\logs`     |

> [!WARNING]
> Releases up to 0.0.5 cannot update themselves to this layout. Their installed
> updater passes the model directory in the old positional form, which the new
> executable deliberately no longer accepts. This is an intentional beta
> compatibility break: repeat the current installation procedure in the
> README instead of running the updater from the old installation. On Windows,
> regenerate the layer models under `%LOCALAPPDATA%\keymap-overlay` first. The
> fresh installation rewrites the login service but leaves the old
> `~/.config/keymap-overlay` or `%USERPROFILE%\.config\keymap-overlay` directory
> in place; delete that directory by hand after the new overlay starts.
>
> The same releases also install one JSON file per layer (`<keyboard>_L<n>.json`)
> under `~/.config/keymap-overlay` on macOS and Linux. This layout instead
> installs one file per keyboard (`<keyboard>.json`) under `~/.cache/keymap-overlay`,
> since the models are a regenerable cache of what a VIAL-flashed device already
> knows, not configuration. `install.sh` itself still copies to
> `~/.config/keymap-overlay` — only the installer's own bookkeeping, not the
> models, since it backs the documented uninstall command and should not
> disappear if a cache directory is cleared. Delete the stale
> `~/.config/keymap-overlay/*_L*.json` files by hand; nothing reads them anymore.
> Windows is unaffected: both concerns already shared `%LOCALAPPDATA%\keymap-overlay`.

On macOS and Linux the MIT terms and the generated third-party notices are
embedded in the executable and printed by `keymap-overlay --license` and
`--third-party-licenses` rather than installed as files. Every release archive
still contains both, and the Windows package installs them beside the
executable.

## Versioning

Tags, Rust crates, and the Python package use the same semantic version. While
the project is below 1.0, a minor release may contain a compatibility change;
patch releases are intended for compatible fixes. The latest release and
`main` are the only supported versions.

Compatibility changes and required migration steps belong in the GitHub
release notes. Please report an unexpected regression through GitHub Issues.

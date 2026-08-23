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
| Linux    | `x86_64`, `arm64`       | GNOME 45+ or Qt 6/LayerShellQt, with systemd user services         |
| Windows  | `x86_64`, `arm64`       | Native Windows overlay installed from PowerShell                   |

Firmware compilation and flashing are supported on macOS and Linux. On a
Windows host, use WSL for firmware and layer-image work; the overlay itself is
native Windows software. Other architectures and init systems are not currently
release targets.

## Installed Layout

Each system uses its own convention for per-user files:

|                       | macOS and Linux                                               | Windows                                  |
| --------------------- | ------------------------------------------------------------- | ---------------------------------------- |
| Executables           | `~/.local/bin`                                                | `%LOCALAPPDATA%\Programs\keymap-overlay` |
| Layer models          | In memory only                                                | In memory only                           |
| Installer bookkeeping | `~/.config/keymap-overlay`                                    | `%LOCALAPPDATA%\keymap-overlay`          |
| Logs                  | journald on Linux; `~/.local/var/log/keymap-overlay` on macOS | `%LOCALAPPDATA%\keymap-overlay\logs`     |

> [!WARNING]
> Firmware built before self-describing Vial metadata was introduced must be
> reflashed. Current installers remove legacy cached model JSON; the runtime
> no longer reads it.

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

# Compatibility Policy

keymap-overlay is currently beta software, tested on the maintainer's systems.
Before 1.0, releases may change installation paths, configuration, generated
assets, or the Raw HID protocol when the change is documented in the release
notes. Backward compatibility is a goal, not yet a guarantee.

## Tested Platforms

Release CI builds and tests these targets:

| Platform | Release architecture    | Runtime expectation                                                   |
| -------- | ----------------------- | --------------------------------------------------------------------- |
| macOS    | Apple silicon (`arm64`) | Current GitHub-hosted macOS runner and Input Monitoring permission    |
| Linux    | `x86_64`                | Wayland with layer shell, or X11/XWayland, plus systemd user services |
| Windows  | `x86_64`                | Native Windows overlay installed from PowerShell                      |

Firmware compilation and flashing are supported on macOS and Linux. On a
Windows host, use WSL for firmware and layer-image work; the overlay itself is
native Windows software. Other architectures, init systems, and Windows on Arm
are not currently release targets.

## Versioning

Tags, Rust crates, and the Python package use the same semantic version. While
the project is below 1.0, a minor release may contain a compatibility change;
patch releases are intended for compatible fixes. The latest release and
`main` are the only supported versions.

Compatibility changes and required migration steps belong in the GitHub
release notes. Please report an unexpected regression through GitHub Issues.

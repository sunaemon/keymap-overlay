# Copyright 2026 sunaemon
# SPDX-License-Identifier: MIT
import tarfile
import zipfile
from pathlib import Path

import pytest

from installer.release.package_release import ReleasePackagingError, package_release


def test_linux_archive_contains_both_renderers_and_gnome_extension(
    tmp_path: Path,
) -> None:
    """Package the complete Linux release payload."""
    root = create_release_tree(tmp_path)

    asset = package_release("Linux", "x86_64", root=root, output_dir=root / "dist")

    with tarfile.open(asset) as archive:
        files = {member.name for member in archive.getmembers() if member.isfile()}
    assert files == {
        "LICENSE",
        "THIRD-PARTY-LICENSES.html",
        "example/LICENSE",
        "gnome-shell/extension.js",
        "keymap-overlay",
        "keymap-overlay-qt",
    }


def test_linux_arm64_archive_uses_native_architecture_name(tmp_path: Path) -> None:
    """Verify that Linux ARM64 archives use the native architecture name."""
    root = create_release_tree(tmp_path)

    asset = package_release("Linux", "arm64", root=root, output_dir=root / "dist")

    assert asset.name == "keymap-overlay-linux-arm64.tar.gz"


def test_macos_archive_contains_the_native_overlay(tmp_path: Path) -> None:
    """Package the complete macOS release payload."""
    root = create_release_tree(tmp_path)

    asset = package_release("macOS", "arm64", root=root, output_dir=root / "dist")

    with tarfile.open(asset) as archive:
        files = {member.name for member in archive.getmembers() if member.isfile()}
    assert files == {
        "LICENSE",
        "THIRD-PARTY-LICENSES.html",
        "example/LICENSE",
        "keymap-overlay",
    }


def test_windows_archive_contains_native_overlay(tmp_path: Path) -> None:
    """Package the complete Windows release payload."""
    root = create_release_tree(tmp_path)

    asset = package_release(
        "Windows",
        "x86_64",
        root=root,
        output_dir=root / "dist",
    )

    with zipfile.ZipFile(asset) as archive:
        files = set(archive.namelist())
    assert files == {
        "LICENSE",
        "THIRD-PARTY-LICENSES.html",
        "example/LICENSE",
        "keymap-overlay.exe",
    }


def test_windows_arm64_archive_uses_native_architecture_name(tmp_path: Path) -> None:
    """Verify that Windows ARM64 archives use the native architecture name."""
    root = create_release_tree(tmp_path)

    asset = package_release(
        "Windows",
        "arm64",
        root=root,
        output_dir=root / "dist",
    )

    assert asset.name == "keymap-overlay-windows-arm64.zip"


def test_unknown_platform_is_rejected(tmp_path: Path) -> None:
    root = create_release_tree(tmp_path)

    with pytest.raises(ReleasePackagingError, match="Unsupported"):
        package_release("Haiku", "x86_64", root=root, output_dir=root / "dist")


def create_release_tree(tmp_path: Path) -> Path:
    """Create the common built files consumed by every package backend."""
    root = tmp_path / "repo"
    write_file(root / "LICENSE.md")
    write_file(root / "firmware/examples/LICENSE")
    write_file(root / "THIRD-PARTY-LICENSES.html")
    write_file(root / "target/release/keymap-overlay")
    write_file(root / "target/release/keymap-overlay-qt")
    write_file(root / "target/release/keymap-overlay.exe")
    write_file(root / "firmware/examples/1/config.json")
    write_file(root / "firmware/examples/1/keyboard.json")
    write_file(root / "overlay/platforms/linux/gnome-shell/extension.js")
    return root


def write_file(path: Path) -> None:
    """Write a fixture file and all of its parent directories."""
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(path.name)

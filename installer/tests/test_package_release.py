# Copyright 2026 sunaemon
# SPDX-License-Identifier: MIT
import subprocess
import tarfile
import zipfile
from pathlib import Path

import pytest

from installer.release.package_release import ReleasePackagingError, package_release


def test_linux_archive_contains_both_renderers_and_gnome_extension(
    tmp_path: Path,
) -> None:
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


def test_macos_archive_contains_the_native_overlay(tmp_path: Path) -> None:
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


def test_windows_archive_contains_wpf_and_dotnet_licenses(tmp_path: Path) -> None:
    root = create_release_tree(tmp_path)
    dotnet_root = create_dotnet_tree(tmp_path)

    asset = package_release(
        "Windows",
        "x86_64",
        root=root,
        output_dir=root / "dist",
        dotnet_root=dotnet_root,
        runner=dotnet_runner,
    )

    with zipfile.ZipFile(asset) as archive:
        files = set(archive.namelist())
    assert files == {
        "DOTNET-LIBRARY-LICENSE.txt",
        "LICENSE",
        "THIRD-PARTY-LICENSES.html",
        "dotnet-runtime-10.0.0-THIRD-PARTY-NOTICES.txt",
        "dotnet-wpf-10.0.0-THIRD-PARTY-NOTICES.txt",
        "example/LICENSE",
        "keymap-overlay.exe",
    }


def test_windows_requires_dotnet_root(tmp_path: Path) -> None:
    root = create_release_tree(tmp_path)

    with pytest.raises(ReleasePackagingError, match="dotnet-root"):
        package_release("Windows", "x86_64", root=root, output_dir=root / "dist")


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
    write_file(root / "target/wpf-publish/keymap-overlay.exe")
    write_file(root / "overlay/platforms/linux/gnome-shell/extension.js")
    return root


def create_dotnet_tree(tmp_path: Path) -> Path:
    """Create the .NET license files shipped in the Windows archive."""
    root = tmp_path / "dotnet"
    write_file(root / "LICENSE.txt")
    write_file(root / "ThirdPartyNotices.txt")
    write_file(
        root
        / "sdk/10.0.100/Sdks/Microsoft.NET.Sdk.WindowsDesktop"
        / "THIRD-PARTY-NOTICES.TXT"
    )
    return root


def dotnet_runner(command: list[str]) -> subprocess.CompletedProcess[str]:
    """Return the pinned SDK and runtime versions used by the test tree."""
    if command == ["dotnet", "--version"]:
        return subprocess.CompletedProcess(command, 0, "10.0.100\n", "")
    if command == ["dotnet", "--list-runtimes"]:
        runtimes = (
            "Microsoft.NETCore.App 10.0.0 [/dotnet/shared]\n"
            "Microsoft.WindowsDesktop.App 10.0.0 [/dotnet/shared]\n"
        )
        return subprocess.CompletedProcess(command, 0, runtimes, "")
    raise AssertionError(f"Unexpected command: {command}")


def write_file(path: Path) -> None:
    """Write a fixture file and all of its parent directories."""
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(path.name)

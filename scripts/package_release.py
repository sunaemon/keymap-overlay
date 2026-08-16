# Copyright 2026 sunaemon
# SPDX-License-Identifier: MIT
import logging
import shutil
import subprocess
import tarfile
import tempfile
import zipfile
from collections.abc import Callable
from pathlib import Path
from typing import Annotated

import typer

from src.util import initialize_logging

logger = logging.getLogger(__name__)

app = typer.Typer()

CommandRunner = Callable[[list[str]], subprocess.CompletedProcess[str]]


class ReleasePackagingError(Exception):
    """Raised when release files cannot be assembled safely."""


@app.command()
def main(
    platform: Annotated[str, typer.Option(help="GitHub Actions runner OS name")],
    architecture: Annotated[str, typer.Option(help="Release architecture name")],
    github_output: Annotated[
        Path, typer.Option(help="GitHub Actions output file to append to")
    ],
    root: Annotated[
        Path, typer.Option(help="Repository root containing the built files")
    ] = Path("."),
    output_dir: Annotated[
        Path, typer.Option(help="Directory in which to create the archive")
    ] = Path("dist"),
    dotnet_root: Annotated[
        Path | None,
        typer.Option(
            help="Root of the bundled .NET installation", envvar="DOTNET_ROOT"
        ),
    ] = None,
) -> None:
    """Package one platform's tested release files and licenses."""
    initialize_logging()
    try:
        asset = package_release(
            platform,
            architecture,
            root=root,
            output_dir=output_dir,
            dotnet_root=dotnet_root,
        )
        with github_output.open("a", encoding="utf-8") as output:
            output.write(f"asset={asset.name}\n")
    except (OSError, ReleasePackagingError, subprocess.SubprocessError):
        logger.exception("Failed to package the %s release", platform)
        raise typer.Exit(code=1) from None


def package_release(
    platform: str,
    architecture: str,
    *,
    root: Path = Path("."),
    output_dir: Path = Path("dist"),
    dotnet_root: Path | None = None,
    runner: CommandRunner | None = None,
) -> Path:
    """Create and return one platform release archive."""
    run = runner or run_command
    output_dir.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(dir=output_dir) as temporary_directory:
        package = Path(temporary_directory) / "package"
        (package / "example").mkdir(parents=True)
        copy_file(root / "LICENSE.md", package / "LICENSE")
        copy_file(root / "example" / "LICENSE", package / "example" / "LICENSE")
        copy_file(
            root / "THIRD-PARTY-LICENSES.html",
            package / "THIRD-PARTY-LICENSES.html",
        )

        if platform == "Linux":
            asset = output_dir / f"keymap-overlay-linux-{architecture}.tar.gz"
            copy_file(
                root / "target/release/keymap-overlay", package / "keymap-overlay"
            )
            copy_file(
                root / "target/release/keymap-overlay-qt",
                package / "keymap-overlay-qt",
            )
            shutil.copytree(root / "linux/gnome-shell", package / "gnome-shell")
            create_tar_archive(package, asset)
        elif platform == "macOS":
            asset = output_dir / f"keymap-overlay-macos-{architecture}.tar.gz"
            copy_file(
                root / "target/release/keymap-overlay", package / "keymap-overlay"
            )
            create_tar_archive(package, asset)
        elif platform == "Windows":
            if dotnet_root is None:
                raise ReleasePackagingError("--dotnet-root is required on Windows")
            asset = output_dir / f"keymap-overlay-windows-{architecture}.zip"
            copy_file(
                root / "target/wpf-publish/keymap-overlay.exe",
                package / "keymap-overlay.exe",
            )
            add_dotnet_licenses(package, dotnet_root, run)
            create_zip_archive(package, asset)
        else:
            raise ReleasePackagingError(f"Unsupported release platform: {platform}")

    logger.info("Created %s", asset)
    return asset


def add_dotnet_licenses(package: Path, dotnet_root: Path, run: CommandRunner) -> None:
    """Add the .NET runtime and WPF license files to a Windows package."""
    sdk_version = run(["dotnet", "--version"]).stdout.strip()
    runtimes = run(["dotnet", "--list-runtimes"]).stdout
    runtime_version = find_runtime_version(runtimes, "Microsoft.NETCore.App")
    wpf_version = find_runtime_version(runtimes, "Microsoft.WindowsDesktop.App")
    copy_file(dotnet_root / "LICENSE.txt", package / "DOTNET-LIBRARY-LICENSE.txt")
    copy_file(
        dotnet_root / "ThirdPartyNotices.txt",
        package / f"dotnet-runtime-{runtime_version}-THIRD-PARTY-NOTICES.txt",
    )
    copy_file(
        dotnet_root
        / "sdk"
        / sdk_version
        / "Sdks"
        / "Microsoft.NET.Sdk.WindowsDesktop"
        / "THIRD-PARTY-NOTICES.TXT",
        package / f"dotnet-wpf-{wpf_version}-THIRD-PARTY-NOTICES.txt",
    )


def find_runtime_version(runtimes: str, name: str) -> str:
    """Return the first installed version of a named .NET runtime."""
    for line in runtimes.splitlines():
        fields = line.split()
        if len(fields) >= 2 and fields[0] == name:
            return fields[1]
    raise ReleasePackagingError(f"Required .NET runtime is missing: {name}")


def copy_file(source: Path, destination: Path) -> None:
    """Copy one required release file, preserving its metadata."""
    shutil.copy2(source, destination)


def create_tar_archive(package: Path, asset: Path) -> None:
    """Create a gzip-compressed tar archive from a package directory."""
    with tarfile.open(asset, "w:gz") as archive:
        for child in sorted(package.iterdir()):
            archive.add(child, arcname=child.name)


def create_zip_archive(package: Path, asset: Path) -> None:
    """Create a ZIP archive from a package directory."""
    with zipfile.ZipFile(asset, "w", compression=zipfile.ZIP_DEFLATED) as archive:
        for path in sorted(item for item in package.rglob("*") if item.is_file()):
            archive.write(path, path.relative_to(package))


def run_command(command: list[str]) -> subprocess.CompletedProcess[str]:
    """Run a release packaging command and capture its text output."""
    return subprocess.run(command, check=True, capture_output=True, text=True)


if __name__ == "__main__":
    app()

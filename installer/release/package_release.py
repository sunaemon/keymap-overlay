# Copyright 2026 sunaemon
# SPDX-License-Identifier: MIT
import logging
import shutil
import tarfile
import tempfile
import zipfile
from pathlib import Path
from typing import Annotated

import typer

from model.src.util import initialize_logging

logger = logging.getLogger(__name__)

app = typer.Typer()


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
) -> None:
    """Package one platform's tested release files and licenses."""
    initialize_logging()
    try:
        asset = package_release(
            platform,
            architecture,
            root=root,
            output_dir=output_dir,
        )
        with github_output.open("a", encoding="utf-8") as output:
            output.write(f"asset={asset.name}\n")
    except (OSError, ReleasePackagingError):
        logger.exception("Failed to package the %s release", platform)
        raise typer.Exit(code=1) from None


def package_release(
    platform: str,
    architecture: str,
    *,
    root: Path = Path("."),
    output_dir: Path = Path("dist"),
) -> Path:
    """Create and return one platform release archive."""
    output_dir.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(dir=output_dir) as temporary_directory:
        package = Path(temporary_directory) / "package"
        (package / "example").mkdir(parents=True)
        copy_file(root / "LICENSE.md", package / "LICENSE")
        copy_file(
            root / "firmware" / "examples" / "LICENSE", package / "example" / "LICENSE"
        )
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
            shutil.copytree(
                root / "overlay/platforms/linux/gnome-shell", package / "gnome-shell"
            )
            create_tar_archive(package, asset)
        elif platform == "macOS":
            asset = output_dir / f"keymap-overlay-macos-{architecture}.tar.gz"
            copy_file(
                root / "target/release/keymap-overlay", package / "keymap-overlay"
            )
            create_tar_archive(package, asset)
        elif platform == "Windows":
            asset = output_dir / f"keymap-overlay-windows-{architecture}.zip"
            copy_file(
                root / "target/release/keymap-overlay.exe",
                package / "keymap-overlay.exe",
            )
            create_zip_archive(package, asset)
        else:
            raise ReleasePackagingError(f"Unsupported release platform: {platform}")

    logger.info("Created %s", asset)
    return asset


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


if __name__ == "__main__":
    app()

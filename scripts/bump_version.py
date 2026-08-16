# Copyright 2026 sunaemon
# SPDX-License-Identifier: MIT
import logging
import re
import subprocess
import tomllib
from collections.abc import Callable
from pathlib import Path
from typing import Annotated

import typer

from src.util import initialize_logging

logger = logging.getLogger(__name__)

app = typer.Typer()

CommandRunner = Callable[[list[str]], subprocess.CompletedProcess[str]]

DEFAULT_CARGO_MANIFEST = Path("Cargo.toml")
DEFAULT_PYTHON_PROJECT = Path("pyproject.toml")
RELEASE_VERSION_PATTERN = re.compile(
    r"^(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)$"
)
SECTION_PATTERN = re.compile(r"^\s*\[([^]]+)]\s*(?:#.*)?$")
VERSION_LINE_PATTERN = re.compile(r'^(\s*version\s*=\s*")([^"]+)("[^\r\n]*)(\r?\n)?$')


class VersionBumpError(Exception):
    """Raised when release metadata cannot be bumped safely."""


@app.command()
def main(
    version: Annotated[str, typer.Argument(help="New X.Y.Z release version")],
) -> None:
    """Bump all release metadata and regenerate derived files."""
    initialize_logging()
    try:
        bump_version(version)
    except (OSError, subprocess.SubprocessError, VersionBumpError):
        logger.exception("Failed to bump the release version to %s", version)
        raise typer.Exit(code=1) from None


def bump_version(
    version: str,
    *,
    cargo_manifest: Path = DEFAULT_CARGO_MANIFEST,
    python_project: Path = DEFAULT_PYTHON_PROJECT,
    runner: CommandRunner | None = None,
) -> None:
    """Bump manifests, refresh lockfiles, and regenerate license notices."""
    target = parse_release_version(version)
    cargo_content = cargo_manifest.read_text(encoding="utf-8")
    python_content = python_project.read_text(encoding="utf-8")
    cargo_version = read_section_version(cargo_content, "workspace.package")
    python_version = read_section_version(python_content, "project")

    if cargo_version != python_version:
        raise VersionBumpError(
            "Cargo.toml and pyproject.toml versions do not match: "
            f"{cargo_version} != {python_version}"
        )
    if target <= parse_release_version(cargo_version):
        raise VersionBumpError(
            f"new version {version} must be greater than current version {cargo_version}"
        )

    cargo_manifest.write_text(
        replace_section_version(cargo_content, "workspace.package", version),
        encoding="utf-8",
    )
    python_project.write_text(
        replace_section_version(python_content, "project", version),
        encoding="utf-8",
    )

    run = runner or run_command
    for command in (
        ["cargo", "check", "--workspace"],
        ["uv", "lock"],
        ["make", "licenses"],
    ):
        logger.info("Running %s", " ".join(command))
        run(command)

    logger.info("Bumped release metadata from %s to %s", cargo_version, version)


def parse_release_version(version: str) -> tuple[int, int, int]:
    """Parse a stable semantic release version."""
    if RELEASE_VERSION_PATTERN.fullmatch(version) is None:
        raise VersionBumpError(
            f"invalid release version {version!r}; expected X.Y.Z without leading zeros"
        )
    major, minor, patch = (int(component) for component in version.split("."))
    return major, minor, patch


def read_section_version(content: str, section: str) -> str:
    """Read a version from one TOML section."""
    try:
        value: object = tomllib.loads(content)
        for component in section.split("."):
            if not isinstance(value, dict):
                raise KeyError(component)
            value = value[component]
        if not isinstance(value, dict):
            raise KeyError(section)
        version = value["version"]
    except (KeyError, tomllib.TOMLDecodeError) as error:
        raise VersionBumpError(f"section [{section}] has no valid version") from error
    if not isinstance(version, str):
        raise VersionBumpError(f"section [{section}] has no valid version")
    return version


def replace_section_version(content: str, section: str, version: str) -> str:
    """Replace only the version assignment in one TOML section."""
    current_section: str | None = None
    replacements = 0
    updated: list[str] = []
    for line in content.splitlines(keepends=True):
        section_match = SECTION_PATTERN.fullmatch(line.rstrip("\r\n"))
        if section_match is not None:
            current_section = section_match.group(1)
        version_match = VERSION_LINE_PATTERN.fullmatch(line)
        if current_section == section and version_match is not None:
            line = (
                f"{version_match.group(1)}{version}{version_match.group(3)}"
                f"{version_match.group(4) or ''}"
            )
            replacements += 1
        updated.append(line)

    if replacements != 1:
        raise VersionBumpError(
            f"expected one version assignment in [{section}], found {replacements}"
        )
    return "".join(updated)


def run_command(command: list[str]) -> subprocess.CompletedProcess[str]:
    """Run one version-bump command."""
    return subprocess.run(command, check=True, text=True)


if __name__ == "__main__":
    app()

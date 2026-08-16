# Copyright 2026 sunaemon
# SPDX-License-Identifier: MIT
import subprocess
from collections.abc import Callable
from pathlib import Path

import pytest

from scripts.bump_version import VersionBumpError, bump_version


def test_bump_version_updates_manifests_and_regenerates_derived_files(
    tmp_path: Path,
) -> None:
    cargo_manifest = tmp_path / "Cargo.toml"
    cargo_manifest.write_text(
        '[package]\nversion = "9.9.9"\n\n[workspace.package]\nversion = "0.0.4"\n',
        encoding="utf-8",
    )
    python_project = tmp_path / "pyproject.toml"
    python_project.write_text(
        '[project]\nversion = "0.0.4"\n\n[tool.example]\nversion = "9.9.9"\n',
        encoding="utf-8",
    )
    commands: list[list[str]] = []

    bump_version(
        "0.0.5",
        cargo_manifest=cargo_manifest,
        python_project=python_project,
        runner=record_commands(commands),
    )

    assert '[package]\nversion = "9.9.9"' in cargo_manifest.read_text()
    assert '[workspace.package]\nversion = "0.0.5"' in cargo_manifest.read_text()
    assert '[project]\nversion = "0.0.5"' in python_project.read_text()
    assert '[tool.example]\nversion = "9.9.9"' in python_project.read_text()
    assert commands == [
        ["cargo", "check", "--workspace"],
        ["uv", "lock"],
        ["make", "licenses"],
    ]


def test_mismatched_manifest_versions_are_rejected_before_writes(
    tmp_path: Path,
) -> None:
    cargo_manifest = tmp_path / "Cargo.toml"
    cargo_manifest.write_text('[workspace.package]\nversion = "0.0.4"\n')
    python_project = tmp_path / "pyproject.toml"
    python_project.write_text('[project]\nversion = "0.0.3"\n')

    with pytest.raises(VersionBumpError, match="do not match"):
        bump_version(
            "0.0.5",
            cargo_manifest=cargo_manifest,
            python_project=python_project,
        )

    assert 'version = "0.0.4"' in cargo_manifest.read_text()
    assert 'version = "0.0.3"' in python_project.read_text()


@pytest.mark.parametrize("version", ["0.0.4", "0.0.3", "v0.0.5", "0.00.5"])
def test_non_release_or_non_increasing_versions_are_rejected(
    tmp_path: Path, version: str
) -> None:
    cargo_manifest = tmp_path / "Cargo.toml"
    cargo_manifest.write_text('[workspace.package]\nversion = "0.0.4"\n')
    python_project = tmp_path / "pyproject.toml"
    python_project.write_text('[project]\nversion = "0.0.4"\n')

    with pytest.raises(VersionBumpError):
        bump_version(
            version,
            cargo_manifest=cargo_manifest,
            python_project=python_project,
        )


def record_commands(
    commands: list[list[str]],
) -> Callable[[list[str]], subprocess.CompletedProcess[str]]:
    """Return a runner that records commands without executing them."""

    def run(command: list[str]) -> subprocess.CompletedProcess[str]:
        commands.append(command)
        return subprocess.CompletedProcess(command, 0)

    return run

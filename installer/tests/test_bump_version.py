# Copyright 2026 sunaemon
# SPDX-License-Identifier: MIT
import subprocess
from collections.abc import Callable
from pathlib import Path

import pytest

from installer.release.bump_version import VersionBumpError, bump_version


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
    gnome_metadata = tmp_path / "metadata.json"
    gnome_metadata.write_text('{"version-name": "0.0.4"}\n', encoding="utf-8")
    commands: list[list[str]] = []

    bump_version(
        "0.0.5",
        cargo_manifest=cargo_manifest,
        python_project=python_project,
        gnome_metadata=gnome_metadata,
        runner=record_commands(commands),
    )

    assert '[package]\nversion = "9.9.9"' in cargo_manifest.read_text()
    assert '[workspace.package]\nversion = "0.0.5"' in cargo_manifest.read_text()
    assert '[project]\nversion = "0.0.5"' in python_project.read_text()
    assert '"version-name": "0.0.5"' in gnome_metadata.read_text()
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
    gnome_metadata = tmp_path / "metadata.json"
    gnome_metadata.write_text('{"version-name": "0.0.4"}\n')

    with pytest.raises(VersionBumpError, match="do not match"):
        bump_version(
            "0.0.5",
            cargo_manifest=cargo_manifest,
            python_project=python_project,
            gnome_metadata=gnome_metadata,
        )

    assert 'version = "0.0.4"' in cargo_manifest.read_text()
    assert 'version = "0.0.3"' in python_project.read_text()


def test_manifest_write_failure_restores_every_original(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    cargo_manifest = tmp_path / "Cargo.toml"
    python_project = tmp_path / "pyproject.toml"
    gnome_metadata = tmp_path / "metadata.json"
    originals = {
        cargo_manifest: '[workspace.package]\nversion = "0.0.4"\n',
        python_project: '[project]\nversion = "0.0.4"\n',
        gnome_metadata: '{"version-name": "0.0.4"}\n',
    }
    for path, content in originals.items():
        path.write_text(content)
    real_write_text = Path.write_text
    failed = False

    def fail_once(
        path: Path,
        content: str,
        encoding: str | None = None,
        errors: str | None = None,
        newline: str | None = None,
    ) -> int:
        nonlocal failed
        if path == gnome_metadata and not failed:
            failed = True
            raise OSError("fixture write failure")
        return real_write_text(path, content, encoding, errors, newline)

    monkeypatch.setattr(Path, "write_text", fail_once)

    with pytest.raises(OSError, match="fixture write failure"):
        bump_version(
            "0.0.5",
            cargo_manifest=cargo_manifest,
            python_project=python_project,
            gnome_metadata=gnome_metadata,
        )

    assert {path: path.read_text() for path in originals} == originals


@pytest.mark.parametrize("version", ["0.0.4", "0.0.3", "v0.0.5", "0.00.5"])
def test_non_release_or_non_increasing_versions_are_rejected(
    tmp_path: Path, version: str
) -> None:
    cargo_manifest = tmp_path / "Cargo.toml"
    cargo_manifest.write_text('[workspace.package]\nversion = "0.0.4"\n')
    python_project = tmp_path / "pyproject.toml"
    python_project.write_text('[project]\nversion = "0.0.4"\n')
    gnome_metadata = tmp_path / "metadata.json"
    gnome_metadata.write_text('{"version-name": "0.0.4"}\n')

    with pytest.raises(VersionBumpError):
        bump_version(
            version,
            cargo_manifest=cargo_manifest,
            python_project=python_project,
            gnome_metadata=gnome_metadata,
        )


def record_commands(
    commands: list[list[str]],
) -> Callable[[list[str]], subprocess.CompletedProcess[str]]:
    """Return a runner that records commands without executing them."""

    def run(command: list[str]) -> subprocess.CompletedProcess[str]:
        commands.append(command)
        return subprocess.CompletedProcess(command, 0)

    return run

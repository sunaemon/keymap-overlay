# Copyright 2026 sunaemon
# SPDX-License-Identifier: MIT
import os
import subprocess
import sys
from pathlib import Path

import pytest


def test_firmware_setup_initializes_only_required_qmk_submodules() -> None:
    """Resolve nested firmware dependencies from the configured keyboards."""
    result = subprocess.run(
        ["make", "-n", "setup-firmware", "OS_FAMILY=linux"],
        check=True,
        capture_output=True,
        text=True,
        cwd=Path(__file__).parents[2],
    )

    assert "firmware.tools.resolve_qmk_submodules" in result.stdout
    assert '"firmware/examples"' in result.stdout
    assert (
        'submodule update --init --checkout --depth 1 "firmware/vendor/vial-qmk"'
        in (result.stdout)
    )
    assert "--recursive)" in result.stdout


def test_recursive_clone_skips_firmware_submodule() -> None:
    """Keep recursive clones from populating every nested QMK dependency."""
    gitmodules = Path(__file__).parents[2] / ".gitmodules"

    result = subprocess.run(
        [
            "git",
            "config",
            "--file",
            str(gitmodules),
            "--get",
            "submodule.firmware/vendor/vial-qmk.update",
        ],
        check=True,
        capture_output=True,
        text=True,
    )

    assert result.stdout.strip() == "none"


def test_qmk_commands_do_not_populate_missing_optional_submodules() -> None:
    """Tell QMK not to undo the selective firmware setup during a build."""
    result = subprocess.run(
        ["make", "-n", "_flash_macos", "KEYBOARD_ID=1"],
        check=True,
        capture_output=True,
        text=True,
        cwd=Path(__file__).parents[2],
    )

    assert "-e SKIP_GIT=yes" in result.stdout


@pytest.mark.skipif(sys.platform == "win32", reason="Makefile paths use POSIX syntax")
def test_install_assets_prunes_removed_layers(tmp_path: Path) -> None:
    """Install only current assets and remove stale layers in both locations."""
    build = tmp_path / "build"
    installed = tmp_path / "installed"
    build.mkdir()
    installed.mkdir()
    current = [build / "7_L0.json", build / "7_L1.json"]
    stale_build = build / "7_L2.json"
    stale_installed = installed / "7_L2.json"
    stale_png = installed / "7_L0.png"
    for path in [*current, stale_build, stale_installed, stale_png]:
        path.write_text("{}", encoding="utf-8")

    subprocess.run(
        [
            "make",
            "_internal_install",
            "KEYBOARD_ID=1",
            "LAYERS=2",
            f"ASSET_BUILD_DIR={build}",
            f"ASSETS={' '.join(map(str, current))}",
            "RENDER_ASSET_DEPS=",
            f"KEYMAP_OVERLAY_DIR={installed}",
            "KEYMAP_PREFIX=7_",
            "ASSET_EXTENSION=json",
            "STALE_ASSET_EXTENSION=png",
        ],
        check=True,
        cwd=Path(__file__).parents[2],
    )

    assert sorted(path.name for path in installed.iterdir()) == [
        "7_L0.json",
        "7_L1.json",
    ]
    assert not stale_build.exists()


@pytest.mark.skipif(sys.platform == "win32", reason="Makefile paths use POSIX syntax")
def test_failed_rp2040_flash_still_unmounts_volume(tmp_path: Path) -> None:
    """Unmount a UF2 volume even when qmk reports a flashing failure."""
    command_log = tmp_path / "commands.log"
    fake_uv = tmp_path / "uv"
    fake_uv.write_text(
        """#!/bin/sh
case "$*" in
  *model.scripts.get_keyboard_metadata*) printf '%s\\n' rp2040 ;;
  *) printf '%s\\n' "$*" >> "$COMMAND_LOG" ;;
esac
""",
        encoding="utf-8",
    )
    fake_uv.chmod(0o755)

    result = subprocess.run(
        [
            "make",
            "_flash_linux",
            "KEYBOARD_ID=1",
            f"UV={fake_uv}",
            "QMK=false",
            "SUDO=",
        ],
        check=False,
        cwd=Path(__file__).parents[2],
        env={**os.environ, "COMMAND_LOG": str(command_log)},
    )

    assert result.returncode != 0
    assert "--unmount" in command_log.read_text(encoding="utf-8")

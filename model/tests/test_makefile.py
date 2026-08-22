# Copyright 2026 sunaemon
# SPDX-License-Identifier: MIT
import os
import shutil
import subprocess
import sys
from pathlib import Path

import pytest

if (make := shutil.which("make")) is None:
    raise RuntimeError("make is required to test the Makefile")
MAKE: str = make


def test_firmware_setup_initializes_only_required_qmk_submodules() -> None:
    """Resolve nested firmware dependencies from the configured keyboards."""
    result = subprocess.run(
        [MAKE, "-n", "setup-firmware", "OS_FAMILY=linux"],
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


@pytest.mark.skipif(sys.platform != "win32", reason="Windows Run-key wiring")
def test_windows_source_install_wires_startup_refresh() -> None:
    """Install the generator and pass both model directories at startup."""
    root = Path(__file__).parents[2]
    install = subprocess.run(
        [MAKE, "-n", "install-overlay", "MAKE=echo"],
        check=True,
        capture_output=True,
        text=True,
        cwd=root,
    )
    service = subprocess.run(
        [MAKE, "-n", "_install_service_windows"],
        check=True,
        capture_output=True,
        text=True,
        cwd=root,
    )

    assert "keymap-overlay-generator.exe" in install.stdout
    assert "no layer JSON models found" not in install.stdout
    assert 'configs="$(cygpath -w ' in service.stdout
    assert service.stdout.count("--keyboard-config-dir") == 2


@pytest.mark.skipif(sys.platform != "win32", reason="Native Windows HID targets")
def test_windows_exposes_native_hid_targets() -> None:
    """Allow device reads and prepared keymap writes without QMK in MSYS2."""
    root = Path(__file__).parents[2]
    install = subprocess.run(
        [MAKE, "-n", "_install_assets_windows", "VIAL=true", "MAKE=echo"],
        check=True,
        capture_output=True,
        text=True,
        cwd=root,
    )
    source_render = subprocess.run(
        [MAKE, "_install_assets_windows", "VIAL=false"],
        check=False,
        capture_output=True,
        text=True,
        cwd=root,
    )
    makefile = (root / "Makefile").read_text(encoding="utf-8")

    assert "_install_assets" in install.stdout
    assert "must run in WSL" not in install.stdout
    assert '"$(FLASHER_BINARY)" --qmk-keymap-json' in makefile
    assert "make prepare-flash-keymap KEYBOARD_ID=$(KEYBOARD_ID)" in makefile
    assert source_render.returncode != 0
    assert "needs QMK source processing" in source_render.stderr


@pytest.mark.skipif(sys.platform == "win32", reason="Firmware builds are unsupported")
def test_qmk_commands_do_not_populate_missing_optional_submodules() -> None:
    """Tell QMK not to undo the selective firmware setup during a build."""
    result = subprocess.run(
        [MAKE, "-n", "_flash_macos", "KEYBOARD_ID=1"],
        check=True,
        capture_output=True,
        text=True,
        cwd=Path(__file__).parents[2],
    )

    assert "-e SKIP_GIT=yes" in result.stdout


@pytest.mark.skipif(sys.platform == "win32", reason="Makefile paths use POSIX syntax")
def test_install_assets_prunes_removed_layers(tmp_path: Path) -> None:
    """Install one consolidated file and remove stale artifacts in both locations."""
    build = tmp_path / "build"
    installed = tmp_path / "installed"
    build.mkdir()
    installed.mkdir()
    # KEYBOARD_ID must name a real firmware/examples/<id> directory (an
    # unconditional Makefile guard reads its config.json), so this reuses
    # bundled keyboard 1 rather than an arbitrary id.
    current = [build / "1_L0.json", build / "1_L1.json"]
    current[0].write_text('{"layer": 0}', encoding="utf-8")
    current[1].write_text('{"layer": 1}', encoding="utf-8")
    # Leftovers from a shrunk layer count and the pre-consolidation format,
    # which the install step must not resurrect.
    stale_build = build / "1_L2.json"
    stale_installed = installed / "1_L2.json"
    stale_png = installed / "1_L0.png"
    for path in [stale_build, stale_installed, stale_png]:
        path.write_text("{}", encoding="utf-8")

    subprocess.run(
        [
            MAKE,
            "_internal_install",
            "VIAL=false",
            "KEYBOARD_ID=1",
            "LAYERS=2",
            f"ASSET_BUILD_DIR={build}",
            f"ASSETS={' '.join(map(str, current))}",
            "RENDER_ASSET_DEPS=",
            f"KEYMAP_OVERLAY_DIR={installed}",
            "ASSET_EXTENSION=json",
            "STALE_ASSET_EXTENSION=png",
        ],
        check=True,
        cwd=Path(__file__).parents[2],
    )

    assert sorted(path.name for path in installed.iterdir()) == ["1.json"]
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
            MAKE,
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

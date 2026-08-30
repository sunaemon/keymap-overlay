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


def test_makefile_keeps_one_public_entry_point_with_concern_fragments() -> None:
    """Keep make public while loading each implementation concern once."""
    root = Path(__file__).parents[2]
    fragments = [
        "config",
        "development",
        "verification",
        "release",
        "overlay",
        "install",
        "firmware",
    ]

    makefile = (root / "Makefile").read_text(encoding="utf-8")

    assert makefile.count("include mk/") == len(fragments)
    for fragment in fragments:
        assert f"include mk/{fragment}.mk\n" in makefile
        assert (root / "mk" / f"{fragment}.mk").is_file()


def test_contract_check_accepts_windows_checkout_line_endings(tmp_path: Path) -> None:
    """Compare generated contracts independently of Git checkout line endings."""
    checked_schema = tmp_path / "display-model.schema.json"
    checked_schema.write_bytes(b'{\r\n  "version": 2\r\n}\r\n')
    generator = tmp_path / "generate-schema"
    generator.write_text(
        "#!/bin/sh\nprintf '{\\n  \"version\": 2\\n}\\n'\n",
        encoding="utf-8",
    )
    generator.chmod(0o755)

    subprocess.run(
        [
            MAKE,
            "check-contracts",
            f"DISPLAY_MODEL_SCHEMA={checked_schema.as_posix()}",
            f"DISPLAY_MODEL_SCHEMA_COMMAND={generator.as_posix()}",
            "CARGO=true",
        ],
        check=True,
        cwd=Path(__file__).parents[2],
    )


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
    """Install one Windows overlay without persistent model arguments."""
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

    assert "keymap-overlay-generator.exe" not in install.stdout
    assert "no layer JSON models found" not in install.stdout
    assert "--asset-dir" not in service.stdout
    assert "--keyboard-config-dir" not in service.stdout


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


@pytest.mark.skipif(sys.platform == "win32", reason="launchd is unavailable")
def test_macos_source_install_retries_transient_launchctl_bootstrap() -> None:
    """Retry the narrow launchd race after replacing a running service."""
    result = subprocess.run(
        [MAKE, "-Bn", "_install_service_macos"],
        check=True,
        capture_output=True,
        text=True,
        cwd=Path(__file__).parents[2],
    )

    assert "launchctl bootstrap failed; retrying" in result.stdout
    assert result.stdout.count('launchctl bootstrap "gui/') == 3


@pytest.mark.skipif(sys.platform == "win32", reason="Makefile paths use POSIX syntax")
@pytest.mark.parametrize("pixels_per_unit", [64, 80])
def test_live_asset_target_runs_the_native_generator(pixels_per_unit: int) -> None:
    """Key generated models by scale and invoke the native Vial reader."""
    result = subprocess.run(
        [
            MAKE,
            "-Bn",
            f"build/1/assets/macos/{pixels_per_unit}/1.json",
            "KEYBOARD_ID=1",
            "OVERLAY_PLATFORM=macos",
            f"PIXELS_PER_UNIT={pixels_per_unit}",
            "CARGO=cargo",
        ],
        check=True,
        capture_output=True,
        text=True,
        cwd=Path(__file__).parents[2],
    )

    assert "cargo run --quiet --package keymap-overlay-generator" in result.stdout
    assert '--keyboard-id "1"' in result.stdout
    assert '--platform "macos"' in result.stdout
    assert f'--pixels-per-unit "{pixels_per_unit}"' in result.stdout


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

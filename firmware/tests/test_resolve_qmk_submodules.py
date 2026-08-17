# Copyright 2026 sunaemon
# SPDX-License-Identifier: MIT
import json
import subprocess
import sys
from pathlib import Path

from firmware.tools.resolve_qmk_submodules import resolve_qmk_submodules


def test_existing_processors_select_only_their_required_submodules(
    tmp_path: Path,
) -> None:
    paths = [
        write_keyboard(tmp_path / "1.json", "RP2040"),
        write_keyboard(tmp_path / "2.json", "STM32F103"),
    ]

    submodules, unknown = resolve_qmk_submodules(paths)

    assert submodules == [
        "lib/chibios",
        "lib/chibios-contrib",
        "lib/lufa",
        "lib/pico-sdk",
    ]
    assert unknown == []


def test_stm32_does_not_require_the_pico_sdk(tmp_path: Path) -> None:
    path = write_keyboard(tmp_path / "keyboard.json", "STM32F103")

    submodules, unknown = resolve_qmk_submodules([path])

    assert submodules == ["lib/chibios", "lib/chibios-contrib", "lib/lufa"]
    assert unknown == []


def test_unknown_or_missing_processors_request_the_safe_fallback(
    tmp_path: Path,
) -> None:
    unknown_path = write_keyboard(tmp_path / "unknown.json", "FutureMCU")
    missing_path = write_keyboard(tmp_path / "missing.json", None)

    submodules, unknown = resolve_qmk_submodules([unknown_path, missing_path])

    assert submodules == []
    assert unknown == ["<missing>", "FutureMCU"]


def test_no_keyboards_requests_the_safe_fallback() -> None:
    submodules, unknown = resolve_qmk_submodules([])

    assert submodules == []
    assert unknown == ["<no keyboards>"]


def test_unknown_processor_warns_and_prints_recursive_fallback(
    tmp_path: Path,
) -> None:
    keyboards_dir = tmp_path / "keyboards"
    write_keyboard(keyboards_dir / "1" / "keyboard.json", "FutureMCU")

    result = subprocess.run(
        [
            sys.executable,
            "-m",
            "firmware.tools.resolve_qmk_submodules",
            str(keyboards_dir),
        ],
        check=True,
        capture_output=True,
        text=True,
    )

    assert result.stdout.strip() == "--recursive"
    assert "WARNING" in result.stderr
    assert "FutureMCU" in result.stderr


def write_keyboard(path: Path, processor: str | None) -> Path:
    """Write the minimal keyboard model needed by the resolver."""
    path.parent.mkdir(parents=True, exist_ok=True)
    keyboard = {
        "keyboard_name": "test",
        "manufacturer": "test",
        "maintainer": "test",
        "usb": {
            "vid": "0x0001",
            "pid": "0x0002",
            "device_version": "0.0.1",
        },
        "matrix_pins": {"rows": ["A0"], "cols": ["A1"]},
        "layouts": {},
    }
    if processor is not None:
        keyboard["processor"] = processor
    path.write_text(json.dumps(keyboard), encoding="utf-8")
    return path

# Copyright 2026 sunaemon
# SPDX-License-Identifier: MIT
import subprocess
import sys
from pathlib import Path

import pytest


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
        cwd=Path(__file__).parents[1],
    )

    assert sorted(path.name for path in installed.iterdir()) == [
        "7_L0.json",
        "7_L1.json",
    ]
    assert not stale_build.exists()

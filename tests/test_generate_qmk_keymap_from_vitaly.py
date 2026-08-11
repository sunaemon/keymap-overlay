# Copyright 2025 sunaemon
# SPDX-License-Identifier: MIT
from pathlib import Path

from scripts.generate_qmk_keymap_from_vitaly import (
    generate_qmk_keymap_from_vitaly,
)

DATA_DIR = Path(__file__).parent / "data"


def test_generate_qmk_keymap_from_vitaly_end_to_end() -> None:
    keymap = generate_qmk_keymap_from_vitaly(
        DATA_DIR / "vitaly.json",
        DATA_DIR / "keyboard.json",
        "LAYOUT",
    )
    assert keymap.version == 1
    assert keymap.layout == "LAYOUT"
    assert keymap.layers == [["KC_A", "KC_B"]]

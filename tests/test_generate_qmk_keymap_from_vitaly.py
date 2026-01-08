# Copyright 2025 sunaemon
# SPDX-License-Identifier: MIT
from pathlib import Path

import pytest

from scripts.generate_qmk_keymap_from_vitaly import (
    flatten_layer,
    generate_qmk_keymap_from_vitaly,
)

DATA_DIR = Path(__file__).parent / "data"


def test_flatten_layer_respects_layout_map() -> None:
    layer_grid = [["KC_A", "KC_B"]]
    layout_map = {(0, 0): 1, (0, 1): 0}
    assert flatten_layer(layer_grid, layout_map) == ["KC_B", "KC_A"]


def test_flatten_layer_empty_layout_map_raises() -> None:
    with pytest.raises(ValueError, match="Layout map is empty"):
        flatten_layer([["KC_A"]], {})


def test_generate_qmk_keymap_from_vitaly_end_to_end() -> None:
    keymap = generate_qmk_keymap_from_vitaly(
        DATA_DIR / "vitaly.json",
        DATA_DIR / "keyboard.json",
        "LAYOUT",
    )
    assert keymap.version == 1
    assert keymap.layout == "LAYOUT"
    assert keymap.layers == [["KC_A", "KC_B"]]

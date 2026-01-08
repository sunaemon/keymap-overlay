# Copyright 2025 sunaemon
# SPDX-License-Identifier: MIT
from pathlib import Path

from scripts.generate_qmk_keymap_from_vitaly import (
    flatten_layer,
    load_layout_map,
)

DATA_DIR = Path(__file__).parent / "data"


def test_load_layout_map_returns_matrix_indices() -> None:
    mapping = load_layout_map(DATA_DIR / "keyboard.json", "LAYOUT")
    assert mapping == {(0, 0): 0, (0, 1): 1}


def test_flatten_layer_respects_layout_map() -> None:
    layer_grid = [["KC_A", "KC_B"]]
    layout_map = {(0, 0): 1, (0, 1): 0}
    assert flatten_layer(layer_grid, layout_map) == ["KC_B", "KC_A"]

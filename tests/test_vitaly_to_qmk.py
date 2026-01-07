# Copyright 2025 sunaemon
# SPDX-License-Identifier: MIT
from pathlib import Path

import pytest

from scripts.generate_qmk_keymap_from_vitaly import (
    flatten_layer,
    load_layout_map,
    parse_layers,
)

DATA_DIR = Path(__file__).parent / "data"


def test_load_layout_map_returns_matrix_indices() -> None:
    mapping = load_layout_map(DATA_DIR / "keyboard.json")
    assert mapping == {(0, 0): 0, (0, 1): 1}


def test_flatten_layer_respects_layout_map() -> None:
    layer_grid = [["KC_A", "KC_B"]]
    layout_map = {(0, 0): 1, (0, 1): 0}
    assert flatten_layer(layer_grid, layout_map) == ["KC_B", "KC_A"]


def test_parse_layers_flattens_grid_with_layout() -> None:
    layout_map = load_layout_map(DATA_DIR / "keyboard.json")
    assert layout_map is not None
    raw_layers = [[["KC_A", "KC_B"]]]
    assert parse_layers(raw_layers, "input", layout_map) == [["KC_A", "KC_B"]]


def test_parse_layers_flattens_grid_without_layout() -> None:
    raw_layers = [[["KC_A", "KC_B"]]]
    assert parse_layers(raw_layers, "input", None) == [["KC_A", "KC_B"]]


def test_parse_layers_rejects_invalid() -> None:
    with pytest.raises(ValueError, match="Invalid input layers"):
        parse_layers(["KC_A"], "input", None)

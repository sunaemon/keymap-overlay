# Copyright 2025 sunaemon
# SPDX-License-Identifier: MIT
import json
from pathlib import Path

from scripts.postprocess_qmk_keymap import postprocess_qmk_keymap


def test_postprocess_qmk_keymap_resolves_custom_and_transparency(
    tmp_path: Path,
) -> None:
    qmk_keymap = {
        "version": 1,
        "layout": "LAYOUT",
        "layers": [["0x0004", "KC_B"], ["KC_TRNS", "KC_TRNS"]],
    }
    custom_keycodes = {"0x0004": "KC_ALPHA"}

    qmk_keymap_path = tmp_path / "qmk-keymap.json"
    custom_keycodes_path = tmp_path / "custom-keycodes.json"

    qmk_keymap_path.write_text(json.dumps(qmk_keymap))
    custom_keycodes_path.write_text(json.dumps(custom_keycodes))

    keymap = postprocess_qmk_keymap(qmk_keymap_path, custom_keycodes_path)

    assert keymap.layers == [["KC_ALPHA", "KC_B"], ["KC_ALPHA", "KC_B"]]


def test_transparency_falls_through_to_the_default_layer(tmp_path: Path) -> None:
    """Holding one momentary layer activates layer 0 and that layer only."""
    qmk_keymap = {
        "version": 1,
        "layout": "LAYOUT",
        "layers": [
            ["KC_ESC", "KC_1", "KC_2"],
            ["KC_TRNS", "KC_F1", "KC_F2"],
            ["KC_TRNS", "KC_TRNS", "KC_ETA"],
        ],
    }
    qmk_keymap_path = tmp_path / "qmk-keymap.json"
    qmk_keymap_path.write_text(json.dumps(qmk_keymap))

    keymap = postprocess_qmk_keymap(qmk_keymap_path, None)

    # Layer 2 must inherit KC_1 from layer 0, not KC_F1 from layer 1.
    assert keymap.layers == [
        ["KC_ESC", "KC_1", "KC_2"],
        ["KC_ESC", "KC_F1", "KC_F2"],
        ["KC_ESC", "KC_1", "KC_ETA"],
    ]


def test_transparent_key_on_the_default_layer_is_left_alone(tmp_path: Path) -> None:
    """Nothing sits below layer 0, so its transparent keys stay transparent."""
    qmk_keymap = {
        "version": 1,
        "layout": "LAYOUT",
        "layers": [["KC_TRNS", "KC_A"], ["KC_TRNS", "KC_TRNS"]],
    }
    qmk_keymap_path = tmp_path / "qmk-keymap.json"
    qmk_keymap_path.write_text(json.dumps(qmk_keymap))

    keymap = postprocess_qmk_keymap(qmk_keymap_path, None)

    assert keymap.layers == [["KC_TRNS", "KC_A"], ["KC_TRNS", "KC_A"]]

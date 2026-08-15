# Copyright 2026 sunaemon
# SPDX-License-Identifier: MIT
import json
from pathlib import Path

import pytest

from scripts.generate_overlay_asset import (
    _parse_encoder_map,
    _resolve_layer,
    build_overlay_model,
    render_png,
)
from src.types import KeycodesJson, QmkKeymapJson


def _write(path: Path, value: object) -> Path:
    path.write_text(json.dumps(value), encoding="utf-8")
    return path


def _keyboard() -> dict:
    return {
        "keyboard_name": "Test",
        "usb": {"vid": "0x0001", "pid": "0x0002", "device_version": "1.0.0"},
        "matrix_pins": {"rows": ["A0"], "cols": ["A1", "A2"]},
        "encoder": {"rotary": [{"pin_a": "B0", "pin_b": "B1"}]},
        "layouts": {
            "LAYOUT": {
                "layout": [
                    {"matrix": [0, 0], "x": 0, "y": 0},
                    {"matrix": [0, 1], "x": 1, "y": 0},
                ]
            }
        },
    }


def test_renders_keys_and_an_encoder_directly_to_rgba(tmp_path: Path) -> None:
    keymap = _write(
        tmp_path / "keymap.json",
        {
            "layout": "LAYOUT",
            "layers": [["KC_A", "KC_MUTE"], ["KC_B", "KC_MUTE"]],
        },
    )
    keyboard = _write(tmp_path / "keyboard.json", _keyboard())
    config = _write(
        tmp_path / "config.json",
        {"qmk_keyboard": "test", "encoders": [{"matrix": [0, 1]}]},
    )
    custom = _write(tmp_path / "custom.json", {})
    keymap_c = tmp_path / "keymap.c"
    keymap_c.write_text(
        """
        const uint16_t PROGMEM encoder_map[2][1][2] = {
          [0] = {ENCODER_CCW_CW(KC_VOLD, KC_VOLU)},
          [1] = {ENCODER_CCW_CW(KC_TRNS, KC_TRNS)},
        };
        """,
        encoding="utf-8",
    )

    args = (
        keymap,
        keyboard,
        config,
        custom,
        "LAYOUT",
        1,
        64,
    )
    model = build_overlay_model(*args, keymap_c=keymap_c)
    image = render_png(*args, keymap_c=keymap_c)

    assert model.version == 1
    assert (model.width, model.height) == (168, 142)
    assert model.keys[0].label == ["B"]
    assert model.encoders[0].counter_clockwise == ["VOL -"]
    assert model.encoders[0].clockwise == ["VOL +"]
    assert model.encoders[0].press == "MUTE"
    assert image.mode == "RGBA"
    assert image.size == (168, 142)
    assert image.getbbox() is not None
    alpha_histogram = image.getchannel("A").histogram()
    assert sum(alpha_histogram[1:255]) > 0


def test_encoder_parser_preserves_nested_keycode_arguments(tmp_path: Path) -> None:
    keymap_c = tmp_path / "keymap.c"
    keymap_c.write_text(
        """
        const uint16_t PROGMEM encoder_map[1][1][2] = {
          [0] = {ENCODER_CCW_CW(LCTL(KC_Z), LT(1, KC_X))},
        };
        """,
        encoding="utf-8",
    )

    assert _parse_encoder_map(keymap_c) == [[["LCTL(KC_Z)", "LT(1, KC_X)"]]]


def test_resolves_display_layer_without_changing_raw_keymap() -> None:
    keymap = QmkKeymapJson(layers=[["0x0004", "KC_B"], ["KC_TRNS", "0x0004"]])
    custom = KeycodesJson({"0x0004": "KC_ALPHA"})

    assert _resolve_layer(keymap, 1, custom) == ["KC_ALPHA", "KC_ALPHA"]
    assert keymap.layers[1] == ["KC_TRNS", "0x0004"]


def test_encoder_placement_count_must_match_keyboard(tmp_path: Path) -> None:
    keymap = _write(tmp_path / "keymap.json", {"layers": [["KC_A", "KC_B"]]})
    keyboard = _write(tmp_path / "keyboard.json", _keyboard())
    config = _write(tmp_path / "config.json", {"qmk_keyboard": "test"})
    custom = _write(tmp_path / "custom.json", {})
    keymap_c = _write(tmp_path / "keymap.c", {})

    with pytest.raises(ValueError, match="encoder placements"):
        render_png(
            keymap,
            keyboard,
            config,
            custom,
            "LAYOUT",
            0,
            keymap_c=keymap_c,
        )

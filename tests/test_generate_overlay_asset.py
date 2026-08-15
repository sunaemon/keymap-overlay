# Copyright 2026 sunaemon
# SPDX-License-Identifier: MIT
import json
from pathlib import Path

import pytest

from scripts.encoder_map import parse_encoder_map
from scripts.generate_overlay_asset import (
    _parse_display_labels,
    _resolve_layer,
    build_overlay_model,
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


def test_builds_keys_and_an_encoder_into_the_shared_model(tmp_path: Path) -> None:
    """Builds keys and an encoder into the shared display model."""
    keymap = _write(
        tmp_path / "keymap.json",
        {
            "layout": "LAYOUT",
            "layers": [["KC_A", "KC_MUTE"], ["KC_ALPHA", "KC_MUTE"]],
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
        /* keymap-overlay-labels
         * KC_ALPHA = α
         */
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

    assert model.version == 2
    assert (model.width, model.height) == (168, 142)
    assert model.keys[0].label == ["α"]
    assert not model.keys[0].transparent
    assert model.encoders[0].counter_clockwise == ["VOL -"]
    assert model.encoders[0].clockwise == ["VOL +"]
    assert model.encoders[0].counter_clockwise_transparent
    assert model.encoders[0].clockwise_transparent
    assert model.encoders[0].press == "MUTE"


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

    assert parse_encoder_map(keymap_c) == [[["LCTL(KC_Z)", "LT(1, KC_X)"]]]


def test_encoder_parser_resolves_symbolic_layer_designators(tmp_path: Path) -> None:
    keymap_c = tmp_path / "keymap.c"
    keymap_c.write_text(
        """
        enum layers { BASE, LOWER };
        const uint16_t PROGMEM encoder_map[2][1][2] = {
          [BASE] = {ENCODER_CCW_CW(KC_VOLD, KC_VOLU)},
          [LOWER] = {ENCODER_CCW_CW(KC_PGDN, KC_PGUP)},
        };
        """,
        encoding="utf-8",
    )

    assert parse_encoder_map(keymap_c) == [
        [["KC_VOLD", "KC_VOLU"]],
        [["KC_PGDN", "KC_PGUP"]],
    ]


@pytest.mark.parametrize("arguments", [", KC_VOLU", "KC_VOLD,", " , "])
def test_encoder_parser_rejects_empty_actions(tmp_path: Path, arguments: str) -> None:
    keymap_c = tmp_path / "keymap.c"
    keymap_c.write_text(
        f"""
        const uint16_t PROGMEM encoder_map[1][1][2] = {{
          [0] = {{ENCODER_CCW_CW({arguments})}},
        }};
        """,
        encoding="utf-8",
    )

    with pytest.raises(ValueError, match="Empty ENCODER_CCW_CW argument"):
        parse_encoder_map(keymap_c)


def test_parses_unicode_display_labels_from_keymap_comment(tmp_path: Path) -> None:
    keymap_c = tmp_path / "keymap.c"
    keymap_c.write_text(
        """
        /* keymap-overlay-labels
         * KC_ALPHA = α
         * KC_LGUI = ⌘
         */
        """,
        encoding="utf-8",
    )

    assert _parse_display_labels(keymap_c) == {
        "KC_ALPHA": "α",
        "KC_LGUI": "⌘",
    }


def test_uses_single_character_custom_keycode_comments_as_labels(
    tmp_path: Path,
) -> None:
    keymap_c = tmp_path / "keymap.c"
    keymap_c.write_text(
        """
        enum custom_keycodes {
          KC_ALPHA = SAFE_RANGE, // α
          KC_BETA,               // β
          KC_INTERNAL            // a longer explanation is not a label
        };
        """,
        encoding="utf-8",
    )

    assert _parse_display_labels(keymap_c) == {
        "KC_ALPHA": "α",
        "KC_BETA": "β",
    }


def test_platform_labels_override_common_labels(tmp_path: Path) -> None:
    keymap_c = tmp_path / "keymap.c"
    keymap_c.write_text(
        """
        /* keymap-overlay-labels
        KC_APP = ☰
        KC_LGUI = GUI
        */
        /* keymap-overlay-labels-macos
        KC_LGUI = ⌘
        */
        /* keymap-overlay-labels-linux
        KC_LGUI = Super
        */
        /* keymap-overlay-labels-windows
        KC_LGUI = ⊞
        */
        """,
        encoding="utf-8",
    )

    assert _parse_display_labels(keymap_c, "macos") == {
        "KC_APP": "☰",
        "KC_LGUI": "⌘",
    }
    assert _parse_display_labels(keymap_c, "linux")["KC_LGUI"] == "Super"
    assert _parse_display_labels(keymap_c, "windows")["KC_LGUI"] == "⊞"


def test_rejects_malformed_display_label(tmp_path: Path) -> None:
    keymap_c = tmp_path / "keymap.c"
    keymap_c.write_text(
        "/* keymap-overlay-labels\nKC_ALPHA α\n*/",
        encoding="utf-8",
    )

    with pytest.raises(ValueError, match="Malformed keymap-overlay label"):
        _parse_display_labels(keymap_c)


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
        build_overlay_model(
            keymap,
            keyboard,
            config,
            custom,
            "LAYOUT",
            0,
            keymap_c=keymap_c,
        )

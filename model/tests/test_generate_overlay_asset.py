# Copyright 2026 sunaemon
# SPDX-License-Identifier: MIT
import json
from pathlib import Path

import pytest

from model.scripts.encoder_map import parse_encoder_map
from model.scripts.generate_overlay_asset import _resolve_layer, build_overlay_model
from model.src.types import KeycodesJson, QmkKeymapJson


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
        enum custom_keycodes {
          KC_ALPHA = SAFE_RANGE, // α
        };
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


def test_a_multi_character_custom_keycode_comment_renders_as_its_glyph(
    tmp_path: Path,
) -> None:
    """A multi-character keymap.c label reaches the rendered key."""
    # Regression test: labels such as "USB-C" used to fall back to the raw
    # keycode name because only single-character comments were accepted.
    keymap = _write(
        tmp_path / "keymap.json",
        {"layout": "LAYOUT", "layers": [["KC_USB_C", "KC_MUTE"]]},
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
        enum custom_keycodes {
          KC_USB_C = SAFE_RANGE, // USB-C
        };
        """,
        encoding="utf-8",
    )

    model = build_overlay_model(
        keymap, keyboard, config, custom, "LAYOUT", 0, 64, keymap_c=keymap_c
    )

    assert model.keys[0].label == ["USB-C"]


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


def test_encoder_parser_resolves_layer_designator_expressions(tmp_path: Path) -> None:
    keymap_c = tmp_path / "keymap.c"
    keymap_c.write_text(
        """
        enum layers { BASE = 0U, LOWER = (BASE + 1) << 1 };
        const uint16_t PROGMEM encoder_map[3][1][2] = {
          [BASE] = {ENCODER_CCW_CW(KC_VOLD, KC_VOLU)},
          [LOWER] = {ENCODER_CCW_CW(KC_PGDN, KC_PGUP)},
        };
        """,
        encoding="utf-8",
    )

    assert parse_encoder_map(keymap_c) == [
        [["KC_VOLD", "KC_VOLU"]],
        [],
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


def test_platform_labels_come_from_built_in_tables(tmp_path: Path) -> None:
    """Common and platform-specific label tables are overlay-owned, not keymap.c."""
    keymap = _write(
        tmp_path / "keymap.json",
        {"layout": "LAYOUT", "layers": [["KC_LGUI", "KC_MUTE"]]},
    )
    keyboard = _write(tmp_path / "keyboard.json", _keyboard())
    config = _write(
        tmp_path / "config.json",
        {"qmk_keyboard": "test", "encoders": [{"matrix": [0, 1]}]},
    )
    custom = _write(tmp_path / "custom.json", {})
    keymap_c = tmp_path / "keymap.c"
    keymap_c.write_text("", encoding="utf-8")

    args = (keymap, keyboard, config, custom, "LAYOUT", 0, 64)

    macos = build_overlay_model(*args, keymap_c=keymap_c, platform="macos")
    assert macos.keys[0].label == ["⌘"]

    linux = build_overlay_model(*args, keymap_c=keymap_c, platform="linux")
    assert linux.keys[0].label == ["Super"]

    windows = build_overlay_model(*args, keymap_c=keymap_c, platform="windows")
    assert windows.keys[0].label == ["⊞"]


def test_custom_keycode_comment_overrides_platform_label(tmp_path: Path) -> None:
    keymap = _write(
        tmp_path / "keymap.json",
        {"layout": "LAYOUT", "layers": [["KC_LGUI", "KC_MUTE"]]},
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
        enum custom_keycodes {
          KC_LGUI = SAFE_RANGE, // ★
        };
        """,
        encoding="utf-8",
    )

    model = build_overlay_model(
        keymap,
        keyboard,
        config,
        custom,
        "LAYOUT",
        0,
        64,
        keymap_c=keymap_c,
        platform="macos",
    )

    assert model.keys[0].label == ["★"]


def test_vial_definition_json_labels_custom_keycodes(tmp_path: Path) -> None:
    """VIAL-mode rendering labels custom keycodes from the device, not keymap.c."""
    keymap = _write(
        tmp_path / "keymap.json",
        {"layout": "LAYOUT", "layers": [["0x7E00"]]},
    )
    keyboard = _write(
        tmp_path / "keyboard.json",
        {
            "keyboard_name": "Test",
            "usb": {"vid": "0x0001", "pid": "0x0002", "device_version": "1.0.0"},
            "matrix_pins": {"rows": ["A0"], "cols": ["A1"]},
            "layouts": {"LAYOUT": {"layout": [{"matrix": [0, 0], "x": 0, "y": 0}]}},
        },
    )
    config = _write(tmp_path / "config.json", {"qmk_keyboard": "test"})
    custom = _write(tmp_path / "custom.json", {"0x7E00": "KC_ALPHA"})
    vitaly_json = _write(tmp_path / "vitaly.json", {"layout": [[["0x7E00"]]]})
    vial_definition_json = _write(
        tmp_path / "vial_definition.json",
        {
            "name": "Test",
            "vendorId": "0xFEED",
            "productId": "0x0001",
            "matrix": {"rows": 1, "cols": 1},
            "layouts": {"keymap": [["0,0"]]},
            "customKeycodes": [{"name": "KC_ALPHA", "title": "", "shortName": "α"}],
        },
    )

    model = build_overlay_model(
        keymap,
        keyboard,
        config,
        custom,
        "LAYOUT",
        0,
        64,
        vitaly_json=vitaly_json,
        vial_definition_json=vial_definition_json,
    )

    assert model.keys[0].label == ["α"]


def test_vial_definition_json_labels_custom_keycodes_from_vitaly_generic_name(
    tmp_path: Path,
) -> None:
    """vitaly has no keyboard-specific names, so it emits generic QK_KB_<n>."""
    keymap = _write(
        tmp_path / "keymap.json",
        {"layout": "LAYOUT", "layers": [["QK_KB_0"]]},
    )
    keyboard = _write(
        tmp_path / "keyboard.json",
        {
            "keyboard_name": "Test",
            "usb": {"vid": "0x0001", "pid": "0x0002", "device_version": "1.0.0"},
            "matrix_pins": {"rows": ["A0"], "cols": ["A1"]},
            "layouts": {"LAYOUT": {"layout": [{"matrix": [0, 0], "x": 0, "y": 0}]}},
        },
    )
    config = _write(tmp_path / "config.json", {"qmk_keyboard": "test"})
    custom = _write(tmp_path / "custom.json", {"0x7E00": "KC_ALPHA"})
    vitaly_json = _write(tmp_path / "vitaly.json", {"layout": [[["QK_KB_0"]]]})
    vial_definition_json = _write(
        tmp_path / "vial_definition.json",
        {
            "name": "Test",
            "vendorId": "0xFEED",
            "productId": "0x0001",
            "matrix": {"rows": 1, "cols": 1},
            "layouts": {"keymap": [["0,0"]]},
            "customKeycodes": [{"name": "KC_ALPHA", "title": "", "shortName": "α"}],
        },
    )

    model = build_overlay_model(
        keymap,
        keyboard,
        config,
        custom,
        "LAYOUT",
        0,
        64,
        vitaly_json=vitaly_json,
        vial_definition_json=vial_definition_json,
    )

    assert model.keys[0].label == ["α"]


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

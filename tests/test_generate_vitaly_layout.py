# Copyright 2026 sunaemon
# SPDX-License-Identifier: MIT
import json
from pathlib import Path

from scripts.generate_vitaly_layout import generate_vitaly_layout

DATA_DIR = Path(__file__).parent / "data"


def _write(path: Path, payload: object) -> Path:
    """Writes payload as JSON to path and returns the path."""
    path.write_text(json.dumps(payload))
    return path


def _empty_keymap_c(tmp_path: Path) -> Path:
    """Create a keymap source without encoder bindings."""
    return _write(tmp_path / "keymap.c", {})


def test_transparency_is_preserved_when_flashing(tmp_path: Path) -> None:
    """KC_TRNS must reach EEPROM intact so layers keep inheriting from layer 0."""
    qmk_keymap = _write(
        tmp_path / "qmk-keymap.json",
        {
            "version": 1,
            "layout": "LAYOUT",
            "layers": [["KC_A", "KC_B"], ["KC_TRNS", "KC_TRNS"]],
        },
    )
    custom_keycodes = _write(tmp_path / "custom-keycodes.json", {})

    vitaly = generate_vitaly_layout(
        qmk_keymap,
        DATA_DIR / "vitaly.json",
        DATA_DIR / "keyboard.json",
        custom_keycodes,
        _empty_keymap_c(tmp_path),
        "LAYOUT",
    )

    # The test matrix is 2x2 but only row 0 is mapped, so row 1 stays KC_NO.
    assert vitaly.layout == [
        [["KC_A", "KC_B"], ["KC_NO", "KC_NO"]],
        [["KC_TRNS", "KC_TRNS"], ["KC_NO", "KC_NO"]],
    ]


def test_custom_keycode_names_are_mapped_back_to_codes(tmp_path: Path) -> None:
    """qmk c2json emits custom keycodes by name; Vial needs the numeric code."""
    qmk_keymap = _write(
        tmp_path / "qmk-keymap.json",
        {"version": 1, "layout": "LAYOUT", "layers": [["KC_ALPHA", "KC_B"]]},
    )
    custom_keycodes = _write(tmp_path / "custom-keycodes.json", {"0x7E40": "KC_ALPHA"})

    vitaly = generate_vitaly_layout(
        qmk_keymap,
        DATA_DIR / "vitaly.json",
        DATA_DIR / "keyboard.json",
        custom_keycodes,
        _empty_keymap_c(tmp_path),
        "LAYOUT",
    )

    assert vitaly.layout == [[["0x7E40", "KC_B"], ["KC_NO", "KC_NO"]]]


def test_encoder_bindings_are_updated_when_flashing(tmp_path: Path) -> None:
    """Encoder actions from keymap.c must replace the device's old bindings."""
    keyboard = json.loads((DATA_DIR / "keyboard.json").read_text())
    keyboard["encoder"] = {
        "rotary": [
            {"pin_a": "A0", "pin_b": "A1"},
            {"pin_a": "A2", "pin_b": "A3"},
        ]
    }
    keyboard_json = _write(tmp_path / "keyboard.json", keyboard)
    qmk_keymap = _write(
        tmp_path / "qmk-keymap.json",
        {"layers": [["KC_A", "KC_B"], ["KC_TRNS", "KC_TRNS"]]},
    )
    vitaly_json = _write(
        tmp_path / "vitaly.json",
        {
            "layout": [[["KC_A"]]],
            "encoder_layout": [[["KC_OLD", "KC_OLD"]]],
        },
    )
    custom_keycodes = _write(tmp_path / "custom-keycodes.json", {"0x7E40": "CUSTOM"})
    keymap_c = tmp_path / "keymap.c"
    keymap_c.write_text(
        """
        const uint16_t PROGMEM encoder_map[2][2][2] = {
            [0] = {ENCODER_CCW_CW(KC_VOLD, KC_VOLU),
                   ENCODER_CCW_CW(CUSTOM, KC_MUTE)},
            [1] = {ENCODER_CCW_CW(KC_TRNS, KC_TRNS)},
        };
        """,
        encoding="utf-8",
    )

    vitaly = generate_vitaly_layout(
        qmk_keymap,
        vitaly_json,
        keyboard_json,
        custom_keycodes,
        keymap_c,
        "LAYOUT",
    )

    assert vitaly.encoder_layout == [
        [["KC_VOLD", "KC_VOLU"], ["0x7E40", "KC_MUTE"]],
        [["KC_TRNS", "KC_TRNS"], ["KC_NO", "KC_NO"]],
    ]


def test_missing_encoder_bindings_clear_old_device_actions(tmp_path: Path) -> None:
    """An absent encoder_map must not leave stale EEPROM actions behind."""
    keyboard = json.loads((DATA_DIR / "keyboard.json").read_text())
    keyboard["encoder"] = {"rotary": [{"pin_a": "A0", "pin_b": "A1"}]}
    keyboard_json = _write(tmp_path / "keyboard.json", keyboard)
    vitaly_json = _write(
        tmp_path / "vitaly.json",
        {
            "layout": [[[]]],
            "encoder_layout": [[["KC_OLD", "KC_OLD"]]],
        },
    )

    vitaly = generate_vitaly_layout(
        _write(tmp_path / "qmk-keymap.json", {"layers": [["KC_A", "KC_B"]]}),
        vitaly_json,
        keyboard_json,
        _write(tmp_path / "custom-keycodes.json", {}),
        _empty_keymap_c(tmp_path),
        "LAYOUT",
    )

    assert vitaly.encoder_layout == [[["KC_NO", "KC_NO"]]]

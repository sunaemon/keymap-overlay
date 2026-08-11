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
        "LAYOUT",
    )

    assert vitaly.layout == [[["0x7E40", "KC_B"], ["KC_NO", "KC_NO"]]]

# Copyright 2025 sunaemon
# SPDX-License-Identifier: MIT
import io
import json
import sys
from pathlib import Path

import pytest
from pydantic import BaseModel, ValidationError

from model.src.types import (
    KeyboardJson,
    KeycodesJson,
    KleKeyProps,
    QmkKeymapJson,
    VialJson,
    VitalyJson,
    print_json,
)

DATA_DIR = Path(__file__).parent / "data"


def _keyboard(**overrides: object) -> dict[str, object]:
    """Builds the fixture keyboard.json payload with overrides applied."""
    keyboard: dict[str, object] = json.loads((DATA_DIR / "keyboard.json").read_text())
    keyboard.update(overrides)
    return keyboard


def _split(*sides: str) -> dict[str, object]:
    """Builds an enabled split configuration with two rows per named side."""
    return {
        "enabled": True,
        "matrix_pins": {
            side: {"rows": ["D4", "D5"], "cols": ["D6", "D7"]} for side in sides
        },
    }


def test_parse_keyboard_json() -> None:
    path = DATA_DIR / "keyboard.json"
    data = json.loads(path.read_text())
    keyboard = KeyboardJson.model_validate(data)
    assert keyboard.keyboard_name == "Test Keyboard"
    assert "LAYOUT" in keyboard.layouts


def test_parse_qmk_keymap_json() -> None:
    path = DATA_DIR / "qmk-keymap.json"
    data = json.loads(path.read_text())
    keymap = QmkKeymapJson.model_validate(data)
    assert keymap.version == 1
    assert keymap.layers is not None
    assert len(keymap.layers) == 2


def test_parse_keycodes_json() -> None:
    path = DATA_DIR / "keycodes.json"
    data = json.loads(path.read_text())
    keycodes = KeycodesJson.model_validate(data)
    assert keycodes.root["0x0004"] == "KC_A"


def test_parse_vial_json() -> None:
    path = DATA_DIR / "vial.json"
    data = json.loads(path.read_text())
    vial = VialJson.model_validate(data)
    assert vial.name == "Test Keyboard"
    assert vial.matrix.rows == 1


def test_parse_vitaly_json() -> None:
    path = DATA_DIR / "vitaly.json"
    data = json.loads(path.read_text())
    vitaly = VitalyJson.model_validate(data)
    assert vitaly.layout is not None
    assert len(vitaly.layout) == 1


def test_matrix_dimensions_of_a_one_piece_keyboard() -> None:
    keyboard = KeyboardJson.model_validate(_keyboard())

    assert keyboard.matrix_dimensions() == (2, 2)


def test_split_keyboards_count_the_rows_of_both_halves() -> None:
    """QMK stacks the secondary half below the primary one in the matrix."""
    keyboard = KeyboardJson.model_validate(_keyboard(split=_split("right")))

    assert keyboard.matrix_dimensions() == (4, 2)


def test_a_disabled_split_does_not_add_rows() -> None:
    split = _split("right")
    split["enabled"] = False

    keyboard = KeyboardJson.model_validate(_keyboard(split=split))

    assert keyboard.matrix_dimensions() == (2, 2)


def test_multiple_split_sides_are_rejected() -> None:
    with pytest.raises(ValidationError, match="multiple split sides not supported"):
        KeyboardJson.model_validate(_keyboard(split=_split("left", "right")))


def test_an_unknown_split_side_is_rejected() -> None:
    with pytest.raises(ValidationError, match="only left and right side split"):
        KeyboardJson.model_validate(_keyboard(split=_split("top")))


def test_a_layout_beyond_the_matrix_is_rejected() -> None:
    """A key mapped outside the matrix would silently drop out of the drawing."""
    layouts = {"LAYOUT": {"layout": [{"x": 0, "y": 0, "matrix": [0, 9]}]}}

    with pytest.raises(ValidationError, match="exceeds matrix dimensions"):
        KeyboardJson.model_validate(_keyboard(layouts=layouts))


def test_a_layout_with_negative_indices_is_rejected() -> None:
    layouts = {"LAYOUT": {"layout": [{"x": 0, "y": 0, "matrix": [-1, 0]}]}}

    with pytest.raises(ValidationError, match="contains negative indices"):
        KeyboardJson.model_validate(_keyboard(layouts=layouts))


def test_an_empty_layout_is_rejected() -> None:
    with pytest.raises(ValidationError, match="Layout LAYOUT mapping is empty"):
        KeyboardJson.model_validate(_keyboard(layouts={"LAYOUT": {"layout": []}}))


def test_layout_mapping_dimensions_pairs_the_mapping_with_the_matrix() -> None:
    keyboard = KeyboardJson.model_validate(_keyboard())

    assert keyboard.layout_mapping_dimensions("LAYOUT") == ([(0, 0), (0, 1)], 2, 2)


def test_a_keyboard_without_encoders_counts_none() -> None:
    keyboard = KeyboardJson.model_validate(_keyboard())

    assert keyboard.encoder_count() == 0


def test_encoders_are_counted() -> None:
    encoder = {
        "rotary": [{"pin_a": "A0", "pin_b": "A1"}, {"pin_a": "A2", "pin_b": "A3"}]
    }

    keyboard = KeyboardJson.model_validate(_keyboard(encoder=encoder))

    assert keyboard.encoder_count() == 2


def test_keycodes_json_rejects_keys_that_are_not_hex() -> None:
    """The map is keyed by wire codes; a name here would corrupt lookups."""
    with pytest.raises(ValidationError, match=r"invalid keys: \['KC_A'\]"):
        KeycodesJson.model_validate({"KC_A": "KC_A"})


def test_kle_key_props_reports_whether_anything_is_set() -> None:
    assert KleKeyProps().has_values() is False
    assert KleKeyProps(x=0.25).has_values() is True
    # Zero is a real offset, not an absent one.
    assert KleKeyProps(x=0.0).has_values() is True


class _Named(BaseModel):
    """A minimal model for exercising print_json's output stream."""

    name: str


# An em dash is the trap here: cp932 encodes the kana but not this.
_NON_ASCII = "かな配列 — L1"


def test_print_json_writes_utf8_whatever_the_stream_encoding_is(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    """A cp932 console must not decide how the generated JSON is encoded."""
    raw = io.BytesIO()
    monkeypatch.setattr(sys, "stdout", io.TextIOWrapper(raw, encoding="cp932"))

    print_json(_Named(name=_NON_ASCII))

    assert json.loads(raw.getvalue().decode("utf-8"))["name"] == _NON_ASCII


def test_print_json_accepts_a_stdout_without_a_binary_buffer(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    """io.StringIO and pytest's capsys have no .buffer to write bytes to."""
    stdout = io.StringIO()
    monkeypatch.setattr(sys, "stdout", stdout)

    print_json(_Named(name=_NON_ASCII))

    assert json.loads(stdout.getvalue())["name"] == _NON_ASCII

# Copyright 2025 sunaemon
# SPDX-License-Identifier: MIT
import json
from pathlib import Path

from src.types import (
    KeyboardJson,
    KeycodesJson,
    KeyToLayerJson,
    QmkKeymapJson,
    VialJson,
    VitalyJson,
)

DATA_DIR = Path(__file__).parent / "data"


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


def test_parse_custom_keycodes_json() -> None:
    path = DATA_DIR / "custom-keycodes.json"
    data = json.loads(path.read_text())
    keycodes = KeycodesJson.model_validate(data)
    assert keycodes.root["0x7E40"] == "SAFE_RANGE"


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


def test_parse_key_to_layer_json() -> None:
    path = DATA_DIR / "key-to-layer.json"
    data = json.loads(path.read_text())
    mapping = KeyToLayerJson.model_validate(data)
    assert mapping.root["f13"] == "L1"

# Copyright 2026 sunaemon
# SPDX-License-Identifier: MIT
import json
from pathlib import Path

from model.scripts.generate_vial import VIAL_ENCODER_LEGEND_SUFFIX, generate_vial
from model.src.types import KleKeyProps

DATA_DIR = Path(__file__).parent / "data"


def test_generate_vial_without_encoders() -> None:
    """Omits the encoder row when the keyboard has no encoders."""
    vial = generate_vial(DATA_DIR / "keyboard.json", "LAYOUT")

    assert vial.layouts.keymap == [["0,0", "0,1"]]


def test_generate_vial_with_encoder_directions(tmp_path: Path) -> None:
    """Appends Vial's directional keys for every rotary encoder."""
    keyboard_data = json.loads((DATA_DIR / "keyboard.json").read_text())
    keyboard_data["encoder"] = {
        "rotary": [
            {"pin_a": "A0", "pin_b": "A1"},
            {"pin_a": "A2", "pin_b": "A3"},
            {"pin_a": "A4", "pin_b": "A5"},
        ]
    }
    keyboard_path = tmp_path / "keyboard.json"
    keyboard_path.write_text(json.dumps(keyboard_data))

    vial = generate_vial(keyboard_path, "LAYOUT")

    assert vial.layouts.keymap[-1] == [
        f"0,0{VIAL_ENCODER_LEGEND_SUFFIX}",
        f"0,1{VIAL_ENCODER_LEGEND_SUFFIX}",
        KleKeyProps(x=0.25),
        f"1,0{VIAL_ENCODER_LEGEND_SUFFIX}",
        f"1,1{VIAL_ENCODER_LEGEND_SUFFIX}",
        KleKeyProps(x=0.25),
        f"2,0{VIAL_ENCODER_LEGEND_SUFFIX}",
        f"2,1{VIAL_ENCODER_LEGEND_SUFFIX}",
    ]

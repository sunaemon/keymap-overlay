# Copyright 2026 sunaemon
# SPDX-License-Identifier: MIT
import json
from pathlib import Path

import pytest
from pydantic import ValidationError

from model.scripts.generate_vial import VIAL_ENCODER_LEGEND_SUFFIX, generate_vial
from model.src.types import KleKeyProps, VialCustomKeycode

DATA_DIR = Path(__file__).parent / "data"


def test_generate_vial_without_encoders() -> None:
    """Omits the encoder row when the keyboard has no encoders."""
    vial = generate_vial(DATA_DIR / "keyboard.json", "LAYOUT")

    assert vial.layouts.keymap == [["0,0", "0,1"]]
    assert vial.customKeycodes is None


def test_generate_vial_embeds_custom_keycodes_from_keymap_c(tmp_path: Path) -> None:
    """The compiled firmware carries each custom keycode's name and glyph."""
    keymap_c = tmp_path / "keymap.c"
    keymap_c.write_text(
        """
        enum custom_keycodes {
          KC_ALPHA = QK_KB_0,    // α
          KC_BETA,               // β
          EIZO_USB_C,            // USB-C
          KC_INTERNAL            // a longer explanation is not a label
        };
        """,
        encoding="utf-8",
    )

    vial = generate_vial(DATA_DIR / "keyboard.json", "LAYOUT", keymap_c=keymap_c)

    assert vial.customKeycodes == [
        VialCustomKeycode(name="KC_ALPHA", shortName="α"),
        VialCustomKeycode(name="KC_BETA", shortName="β"),
        VialCustomKeycode(name="EIZO_USB_C", shortName="USB-C"),
        VialCustomKeycode(name="KC_INTERNAL", shortName=""),
    ]


def test_generate_vial_matches_the_rust_contract_fixture() -> None:
    """The checked-in Vial definition is the Python-to-Rust boundary."""
    vial = generate_vial(
        DATA_DIR / "keyboard.json",
        "LAYOUT",
        keymap_c=DATA_DIR / "contract-keymap.c",
        keyboard_config=DATA_DIR / "contract-config.json",
        keyboard_id=7,
        pixels_per_unit=64,
    )
    expected = json.loads((DATA_DIR / "vial-contract.json").read_text(encoding="utf-8"))

    assert vial.model_dump(mode="json", exclude_none=True) == expected


@pytest.mark.parametrize("base", ["SAFE_RANGE", "QK_USER_0"])
def test_generate_vial_rejects_a_non_keyboard_custom_keycode_base(
    tmp_path: Path, base: str
) -> None:
    keymap_c = tmp_path / "keymap.c"
    keymap_c.write_text(
        f"enum custom_keycodes {{ KC_ALPHA = {base} }};",
        encoding="utf-8",
    )

    with pytest.raises(ValueError, match="must start at QK_KB_0"):
        generate_vial(DATA_DIR / "keyboard.json", "LAYOUT", keymap_c=keymap_c)


def test_generate_vial_rejects_an_implicit_custom_keycode_base(tmp_path: Path) -> None:
    keymap_c = tmp_path / "keymap.c"
    keymap_c.write_text(
        "enum custom_keycodes { KC_ALPHA };",
        encoding="utf-8",
    )

    with pytest.raises(ValueError, match="must start at QK_KB_0"):
        generate_vial(DATA_DIR / "keyboard.json", "LAYOUT", keymap_c=keymap_c)


def test_generate_vial_embeds_self_describing_overlay_metadata(tmp_path: Path) -> None:
    config = tmp_path / "config.json"
    config.write_text('{"qmk_keyboard":"test/keyboard"}', encoding="utf-8")

    vial = generate_vial(
        DATA_DIR / "keyboard.json",
        "LAYOUT",
        keyboard_config=config,
        keyboard_id=7,
        pixels_per_unit=48,
    )

    assert vial.keymapOverlay is not None
    assert vial.keymapOverlay.keyboardId == 7
    assert vial.keymapOverlay.layoutName == "LAYOUT"
    assert vial.keymapOverlay.pixelsPerUnit == 48
    assert vial.keymapOverlay.keyboard.keyboard_name == "Test Keyboard"


@pytest.mark.parametrize("keyboard_id", [-1, 256])
def test_generate_vial_rejects_keyboard_ids_outside_the_protocol_byte(
    tmp_path: Path, keyboard_id: int
) -> None:
    config = tmp_path / "config.json"
    config.write_text('{"qmk_keyboard":"test/keyboard"}', encoding="utf-8")

    with pytest.raises(ValidationError, match="keyboardId"):
        generate_vial(
            DATA_DIR / "keyboard.json",
            "LAYOUT",
            keyboard_config=config,
            keyboard_id=keyboard_id,
        )


@pytest.mark.parametrize("keyboard_id", [0, 255])
def test_generate_vial_accepts_keyboard_ids_at_protocol_boundaries(
    tmp_path: Path, keyboard_id: int
) -> None:
    config = tmp_path / "config.json"
    config.write_text('{"qmk_keyboard":"test/keyboard"}', encoding="utf-8")

    vial = generate_vial(
        DATA_DIR / "keyboard.json",
        "LAYOUT",
        keyboard_config=config,
        keyboard_id=keyboard_id,
    )

    assert vial.keymapOverlay is not None
    assert vial.keymapOverlay.keyboardId == keyboard_id


def test_generate_vial_accepts_a_keymap_without_custom_keycodes(
    tmp_path: Path,
) -> None:
    keymap_c = tmp_path / "keymap.c"
    keymap_c.write_text("#include QMK_KEYBOARD_H\n", encoding="utf-8")

    vial = generate_vial(DATA_DIR / "keyboard.json", "LAYOUT", keymap_c=keymap_c)

    assert vial.customKeycodes == []


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

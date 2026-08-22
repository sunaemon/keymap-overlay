# Copyright 2026 sunaemon
# SPDX-License-Identifier: MIT
import json
from pathlib import Path

import pytest

from model.scripts.generate_custom_keycodes import generate_custom_keycodes


def _write_keycodes(
    tmp_path: Path,
    safe_range: str | None = "0x7E40",
    safe_range_name: str = "SAFE_RANGE",
) -> Path:
    """Writes a keycodes.json holding SAFE_RANGE, or omitting it when None."""
    keycodes = {"0x0004": "KC_A"}
    if safe_range is not None:
        keycodes[safe_range] = safe_range_name
    path = tmp_path / "keycodes.json"
    path.write_text(json.dumps(keycodes), encoding="utf-8")
    return path


def _write_keymap(tmp_path: Path, body: str) -> Path:
    """Writes a keymap.c containing body as the custom_keycodes enum."""
    path = tmp_path / "keymap.c"
    path.write_text(
        f'#include QMK_KEYBOARD_H\n\nenum custom_keycodes {{{body}}};\n\nconst char *name = "x";\n',
        encoding="utf-8",
    )
    return path


def _write_vial_definition(tmp_path: Path, names: list[str]) -> Path:
    """Writes a device-fetched Vial definition holding customKeycodes."""
    path = tmp_path / "vial_definition.json"
    path.write_text(
        json.dumps(
            {
                "name": "Test",
                "vendorId": "0xFEED",
                "productId": "0x0001",
                "matrix": {"rows": 1, "cols": 1},
                "layouts": {"keymap": [["0,0"]]},
                "customKeycodes": [{"name": name} for name in names],
            }
        ),
        encoding="utf-8",
    )
    return path


def test_keycodes_are_numbered_upward_from_safe_range(tmp_path: Path) -> None:
    """The first entry anchors at SAFE_RANGE and the rest follow the enum order."""
    keymap_c = _write_keymap(tmp_path, "\n  KC_ALPHA = SAFE_RANGE,\n  KC_BETA,\n")

    keycodes = generate_custom_keycodes(
        keymap_c=keymap_c, keycodes_json=_write_keycodes(tmp_path)
    )

    assert keycodes.root == {"0x7E40": "KC_ALPHA", "0x7E41": "KC_BETA"}


def test_qk_user_0_is_accepted_as_the_current_safe_range_name(
    tmp_path: Path,
) -> None:
    keymap_c = _write_keymap(tmp_path, " KC_ALPHA = SAFE_RANGE ")

    keycodes = generate_custom_keycodes(
        keymap_c=keymap_c,
        keycodes_json=_write_keycodes(tmp_path, safe_range_name="QK_USER_0"),
    )

    assert keycodes.root == {"0x7E40": "KC_ALPHA"}


def test_qk_user_0_is_accepted_as_the_keymap_anchor(tmp_path: Path) -> None:
    """QK_USER_0 is also valid in the keymap's custom-keycode enum."""
    keymap_c = _write_keymap(tmp_path, " KC_ALPHA = QK_USER_0, KC_BETA ")

    keycodes = generate_custom_keycodes(
        keymap_c=keymap_c, keycodes_json=_write_keycodes(tmp_path)
    )

    assert keycodes.root == {"0x7E40": "KC_ALPHA", "0x7E41": "KC_BETA"}


def test_qk_kb_0_is_accepted_as_the_keymap_anchor(tmp_path: Path) -> None:
    """QK_KB_0 is Vial's own base for a keyboard's custom keycodes."""
    keymap_c = _write_keymap(tmp_path, " KC_ALPHA = QK_KB_0, KC_BETA ")

    keycodes = generate_custom_keycodes(
        keymap_c=keymap_c,
        keycodes_json=_write_keycodes(
            tmp_path, safe_range="0x7E00", safe_range_name="QK_KB_0"
        ),
    )

    assert keycodes.root == {"0x7E00": "KC_ALPHA", "0x7E01": "KC_BETA"}


def test_comments_do_not_become_keycodes(tmp_path: Path) -> None:
    """Real keymaps annotate every entry with the glyph it types."""
    keymap_c = _write_keymap(
        tmp_path,
        "\n  KC_ALPHA = SAFE_RANGE, // α\n  KC_BETA, /* β, and a comma inside */\n  KC_GAMMA, // γ\n",
    )

    keycodes = generate_custom_keycodes(
        keymap_c=keymap_c, keycodes_json=_write_keycodes(tmp_path)
    )

    assert keycodes.root == {
        "0x7E40": "KC_ALPHA",
        "0x7E41": "KC_BETA",
        "0x7E42": "KC_GAMMA",
    }


def test_an_enum_without_a_trailing_comma_is_accepted(tmp_path: Path) -> None:
    keymap_c = _write_keymap(tmp_path, " DUMMY_KEY = SAFE_RANGE ")

    keycodes = generate_custom_keycodes(
        keymap_c=keymap_c, keycodes_json=_write_keycodes(tmp_path)
    )

    assert keycodes.root == {"0x7E40": "DUMMY_KEY"}


def test_explicit_keycode_assignment_is_rejected(tmp_path: Path) -> None:
    """Hand-assigned values would desynchronize the firmware and the drawings."""
    keymap_c = _write_keymap(
        tmp_path, "\n  KC_ALPHA = SAFE_RANGE,\n  KC_BETA = 0x7F00,\n"
    )

    with pytest.raises(
        ValueError, match="Explicit keycode assignment is not supported"
    ):
        generate_custom_keycodes(
            keymap_c=keymap_c, keycodes_json=_write_keycodes(tmp_path)
        )


def test_a_keymap_without_the_enum_has_no_custom_keycodes(tmp_path: Path) -> None:
    keymap_c = tmp_path / "keymap.c"
    keymap_c.write_text("#include QMK_KEYBOARD_H\n")

    keycodes = generate_custom_keycodes(
        keymap_c=keymap_c, keycodes_json=_write_keycodes(tmp_path)
    )

    assert keycodes.root == {}


def test_a_mid_enum_base_reset_is_rejected(tmp_path: Path) -> None:
    keymap_c = _write_keymap(tmp_path, "KC_ALPHA = SAFE_RANGE, KC_BETA = SAFE_RANGE")

    with pytest.raises(ValueError, match="only be assigned to the first entry"):
        generate_custom_keycodes(
            keymap_c=keymap_c, keycodes_json=_write_keycodes(tmp_path)
        )


def test_keycodes_without_safe_range_are_rejected(tmp_path: Path) -> None:
    """Without SAFE_RANGE there is no anchor, so numbering would be invented."""
    keymap_c = _write_keymap(tmp_path, " KC_ALPHA = SAFE_RANGE ")

    with pytest.raises(ValueError, match=r"None of \[.*\] found"):
        generate_custom_keycodes(
            keymap_c=keymap_c,
            keycodes_json=_write_keycodes(tmp_path, safe_range=None),
        )


def test_neither_source_is_rejected(tmp_path: Path) -> None:
    with pytest.raises(ValueError, match="Provide keymap_c and keycodes_json"):
        generate_custom_keycodes()


def test_vial_definition_numbers_custom_keycodes_from_qk_kb_0(
    tmp_path: Path,
) -> None:
    """A device's embedded definition has no numeric base; Vial fixes it at QK_KB_0."""
    vial_definition_json = _write_vial_definition(tmp_path, ["KC_ALPHA", "KC_BETA"])

    keycodes = generate_custom_keycodes(vial_definition_json=vial_definition_json)

    assert keycodes.root == {"0x7E00": "KC_ALPHA", "0x7E01": "KC_BETA"}

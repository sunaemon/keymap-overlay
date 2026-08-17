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


def test_keycodes_are_numbered_upward_from_safe_range(tmp_path: Path) -> None:
    """The first entry anchors at SAFE_RANGE and the rest follow the enum order."""
    keymap_c = _write_keymap(tmp_path, "\n  KC_ALPHA = SAFE_RANGE,\n  KC_BETA,\n")

    keycodes = generate_custom_keycodes(keymap_c, _write_keycodes(tmp_path))

    assert keycodes.root == {"0x7E40": "KC_ALPHA", "0x7E41": "KC_BETA"}


def test_qk_user_0_is_accepted_as_the_current_safe_range_name(
    tmp_path: Path,
) -> None:
    keymap_c = _write_keymap(tmp_path, " KC_ALPHA = SAFE_RANGE ")

    keycodes = generate_custom_keycodes(
        keymap_c,
        _write_keycodes(tmp_path, safe_range_name="QK_USER_0"),
    )

    assert keycodes.root == {"0x7E40": "KC_ALPHA"}


def test_qk_user_0_is_accepted_as_the_keymap_anchor(tmp_path: Path) -> None:
    """QK_USER_0 is also valid in the keymap's custom-keycode enum."""
    keymap_c = _write_keymap(tmp_path, " KC_ALPHA = QK_USER_0, KC_BETA ")

    keycodes = generate_custom_keycodes(keymap_c, _write_keycodes(tmp_path))

    assert keycodes.root == {"0x7E40": "KC_ALPHA", "0x7E41": "KC_BETA"}


def test_comments_do_not_become_keycodes(tmp_path: Path) -> None:
    """Real keymaps annotate every entry with the glyph it types."""
    keymap_c = _write_keymap(
        tmp_path,
        "\n  KC_ALPHA = SAFE_RANGE, // α\n  KC_BETA, /* β, and a comma inside */\n  KC_GAMMA, // γ\n",
    )

    keycodes = generate_custom_keycodes(keymap_c, _write_keycodes(tmp_path))

    assert keycodes.root == {
        "0x7E40": "KC_ALPHA",
        "0x7E41": "KC_BETA",
        "0x7E42": "KC_GAMMA",
    }


def test_an_enum_without_a_trailing_comma_is_accepted(tmp_path: Path) -> None:
    keymap_c = _write_keymap(tmp_path, " DUMMY_KEY = SAFE_RANGE ")

    keycodes = generate_custom_keycodes(keymap_c, _write_keycodes(tmp_path))

    assert keycodes.root == {"0x7E40": "DUMMY_KEY"}


def test_explicit_keycode_assignment_is_rejected(tmp_path: Path) -> None:
    """Hand-assigned values would desynchronize the firmware and the drawings."""
    keymap_c = _write_keymap(
        tmp_path, "\n  KC_ALPHA = SAFE_RANGE,\n  KC_BETA = 0x7F00,\n"
    )

    with pytest.raises(
        ValueError, match="Explicit keycode assignment is not supported"
    ):
        generate_custom_keycodes(keymap_c, _write_keycodes(tmp_path))


def test_a_keymap_without_the_enum_is_rejected(tmp_path: Path) -> None:
    keymap_c = tmp_path / "keymap.c"
    keymap_c.write_text("#include QMK_KEYBOARD_H\n")

    with pytest.raises(ValueError, match="enum custom_keycodes not found"):
        generate_custom_keycodes(keymap_c, _write_keycodes(tmp_path))


def test_keycodes_without_safe_range_are_rejected(tmp_path: Path) -> None:
    """Without SAFE_RANGE there is no anchor, so numbering would be invented."""
    keymap_c = _write_keymap(tmp_path, " KC_ALPHA = SAFE_RANGE ")

    with pytest.raises(ValueError, match="SAFE_RANGE not found"):
        generate_custom_keycodes(keymap_c, _write_keycodes(tmp_path, safe_range=None))

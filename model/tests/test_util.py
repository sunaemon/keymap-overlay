# Copyright 2026 sunaemon
# SPDX-License-Identifier: MIT
from pathlib import Path

import pytest

from model.src.util import (
    load_layout_keys,
    parse_hex_keycode,
    parse_keycode_value,
    strip_c_comments,
)

DATA_DIR = Path(__file__).parent / "data"


def test_strip_c_comments_removes_both_comment_styles() -> None:
    """Keycode enums carry trailing legends that must not become entries."""
    source = "KC_ALPHA = SAFE_RANGE, // α\nKC_BETA, /* β\n spans lines */\nKC_GAMMA,"

    assert strip_c_comments(source) == "KC_ALPHA = SAFE_RANGE, \nKC_BETA, \nKC_GAMMA,"


def test_strip_c_comments_leaves_comment_free_text_alone() -> None:
    assert strip_c_comments("KC_A, KC_B") == "KC_A, KC_B"


@pytest.mark.parametrize(
    ("key", "expected"),
    [
        ("0x0004", 4),
        ("0X0004", 4),
        ("0x7E40", 0x7E40),
        ("0x7e40", 0x7E40),
        # Decimal input is not hex, and neither is a malformed literal.
        ("4", None),
        ("0xZZ", None),
        ("", None),
    ],
)
def test_parse_hex_keycode(key: str, expected: int | None) -> None:
    assert parse_hex_keycode(key) == expected


@pytest.mark.parametrize(
    ("key", "expected"),
    [
        ("0x0004", 4),
        ("4", 4),
        ("0", 0),
        # A negative sign makes isdigit() false, so it is not a keycode.
        ("-4", None),
        ("KC_A", None),
        ("", None),
    ],
)
def test_parse_keycode_value_accepts_hex_and_decimal(
    key: str, expected: int | None
) -> None:
    assert parse_keycode_value(key) == expected


def test_load_layout_keys_returns_the_named_layout() -> None:
    keys = load_layout_keys(DATA_DIR / "keyboard.json", "LAYOUT")

    assert [key.matrix for key in keys] == [(0, 0), (0, 1)]


def test_load_layout_keys_rejects_an_unknown_layout() -> None:
    with pytest.raises(ValueError, match="Layout LAYOUT_MISSING not found"):
        load_layout_keys(DATA_DIR / "keyboard.json", "LAYOUT_MISSING")

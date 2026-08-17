# Copyright 2026 sunaemon
# SPDX-License-Identifier: MIT
from collections.abc import Callable
from pathlib import Path

import pytest

from model.scripts import generate_keycodes as generate_keycodes_module
from model.scripts.generate_keycodes import _latest_qmk_version, generate_keycodes
from model.src.types import KeycodesJson, QmkKeycodesSpec

GenerateWithSpec = Callable[[dict[str, dict[str, object]]], KeycodesJson]

QMK_DIR = Path("firmware/vendor/vial-qmk")


@pytest.fixture
def generate_with_spec(monkeypatch: pytest.MonkeyPatch) -> GenerateWithSpec:
    """Replaces the QMK spec loader so these tests need no firmware checkout."""

    def run(keycodes: dict[str, dict[str, object]]) -> KeycodesJson:
        spec = QmkKeycodesSpec.model_validate({"keycodes": keycodes})
        monkeypatch.setattr(
            generate_keycodes_module,
            "_read_latest_qmk_spec",
            lambda _: spec,
        )
        return generate_keycodes(QMK_DIR)

    return run


def test_a_preferred_alias_wins_over_the_canonical_name(
    generate_with_spec: GenerateWithSpec,
) -> None:
    """Drawings read better with the short names QMK keymaps actually use."""
    keycodes = generate_with_spec(
        {"0x0001": {"key": "KC_TRANSPARENT", "aliases": ["KC_TRNS", "_______"]}}
    )

    assert keycodes.root == {"0x0001": "KC_TRNS"}


def test_kc_no_wins_over_a_shorter_unpreferred_alias(
    generate_with_spec: GenerateWithSpec,
) -> None:
    """KC_NO outranks its aliases even when they are shorter."""
    keycodes = generate_with_spec(
        {"0x0000": {"key": "KC_NO", "aliases": ["NO", "XXXXXXX"]}}
    )

    assert keycodes.root == {"0x0000": "KC_NO"}


def test_the_shortest_name_wins_when_none_is_preferred(
    generate_with_spec: GenerateWithSpec,
) -> None:
    keycodes = generate_with_spec(
        {"0x0055": {"key": "KC_KP_ASTERISK", "aliases": ["KC_PAST"]}}
    )

    assert keycodes.root == {"0x0055": "KC_PAST"}


def test_a_keycode_without_aliases_keeps_its_name(
    generate_with_spec: GenerateWithSpec,
) -> None:
    keycodes = generate_with_spec({"0x0004": {"key": "KC_A"}})

    assert keycodes.root == {"0x0004": "KC_A"}


def test_codes_are_sorted_numerically_and_zero_padded(
    generate_with_spec: GenerateWithSpec,
) -> None:
    """The output is a lookup table, so a stable canonical form matters."""
    keycodes = generate_with_spec(
        {
            "0x7e40": {"key": "KC_ALPHA"},
            "0x4": {"key": "KC_A"},
            "0x0005": {"key": "KC_B"},
        }
    )

    assert list(keycodes.root.items()) == [
        ("0x0004", "KC_A"),
        ("0x0005", "KC_B"),
        ("0x7E40", "KC_ALPHA"),
    ]


def test_unparsable_codes_are_skipped(
    generate_with_spec: GenerateWithSpec,
) -> None:
    """QMK ships spec entries this tool cannot place; they are not fatal."""
    keycodes = generate_with_spec({"KC_A": {"key": "KC_A"}, "0x0005": {"key": "KC_B"}})

    assert keycodes.root == {"0x0005": "KC_B"}


def test_the_first_qmk_keycode_spec_version_is_the_latest() -> None:
    assert _latest_qmk_version(["0.0.7", "0.0.6", "0.0.1"]) == "0.0.7"


@pytest.mark.parametrize("versions", [None, []])
def test_missing_qmk_keycode_spec_versions_are_rejected(
    versions: list[str] | None,
) -> None:
    with pytest.raises(ValueError, match="No QMK keycodes versions found"):
        _latest_qmk_version(versions)

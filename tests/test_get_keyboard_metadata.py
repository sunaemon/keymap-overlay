# Copyright 2026 sunaemon
# SPDX-License-Identifier: MIT
import json
from pathlib import Path

import pytest

from scripts.get_keyboard_metadata import (
    MetadataField,
    format_usb_id,
    get_keyboard_metadata,
)
from src.types import JSONParseError

EXAMPLE_DIR = Path(__file__).parents[1] / "example"


def test_reads_the_platform_metadata() -> None:
    """The Makefile receives normalized, validated values."""
    keyboard_json = EXAMPLE_DIR / "1" / "keyboard.json"

    assert get_keyboard_metadata(keyboard_json, MetadataField.BOOTLOADER) == "rp2040"
    assert get_keyboard_metadata(keyboard_json, MetadataField.VID) == "355d"
    assert get_keyboard_metadata(keyboard_json, MetadataField.PID) == "1001"


def test_formats_short_usb_ids_for_udev() -> None:
    assert format_usb_id("0x2a", "VID") == "002a"


@pytest.mark.parametrize("value", ["not-hex", "0x10000", "-1"])
def test_rejects_invalid_usb_ids(value: str) -> None:
    with pytest.raises(ValueError, match="USB VID"):
        format_usb_id(value, "VID")


def test_invalid_keyboard_json_fails_instead_of_printing_an_empty_value(
    tmp_path: Path,
) -> None:
    keyboard_json = tmp_path / "keyboard.json"
    keyboard_json.write_text(json.dumps({"usb": {"vid": "0x1", "pid": "0x2"}}))

    with pytest.raises(JSONParseError, match="keyboard.json"):
        get_keyboard_metadata(keyboard_json, MetadataField.VID)

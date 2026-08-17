# Copyright 2026 sunaemon
# SPDX-License-Identifier: MIT
import logging
from enum import Enum
from pathlib import Path
from typing import Annotated

import typer

from model.src.types import KeyboardJson, parse_json
from model.src.util import initialize_logging

logger = logging.getLogger(__name__)

app = typer.Typer()


class MetadataField(str, Enum):
    """A keyboard.json value consumed by the Makefile."""

    BOOTLOADER = "bootloader"
    VID = "vid"
    PID = "pid"


@app.command()
def main(
    keyboard_json: Annotated[Path, typer.Argument(help="Path to keyboard.json")],
    field: Annotated[MetadataField, typer.Argument(help="Value to read")],
) -> None:
    """Print one validated keyboard.json value for the Makefile."""
    initialize_logging()
    try:
        print(get_keyboard_metadata(keyboard_json, field))
    except Exception:
        logger.exception("Failed to read %s from %s", field.value, keyboard_json)
        raise typer.Exit(code=1) from None


def get_keyboard_metadata(keyboard_json: Path, field: MetadataField) -> str:
    """Return one validated keyboard.json value used by a platform workflow."""
    keyboard = parse_json(KeyboardJson, keyboard_json)
    match field:
        case MetadataField.BOOTLOADER:
            return keyboard.bootloader or ""
        case MetadataField.VID:
            return format_usb_id(keyboard.usb.vid, "VID")
        case MetadataField.PID:
            return format_usb_id(keyboard.usb.pid, "PID")


def format_usb_id(value: str, name: str) -> str:
    """Return a USB ID as the four lower-case hexadecimal digits udev uses."""
    try:
        number = int(value, 16)
    except ValueError as error:
        raise ValueError(f"USB {name} is not hexadecimal: {value!r}") from error
    if not 0 <= number <= 0xFFFF:
        raise ValueError(f"USB {name} is outside 0x0000..0xffff: {value!r}")
    return f"{number:04x}"


if __name__ == "__main__":
    app()

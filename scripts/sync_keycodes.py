#!/usr/bin/env python3
# Copyright 2025 sunaemon
# SPDX-License-Identifier: MIT
import re
from pathlib import Path

import typer

from src.types import CustomKeycodesJson, KeycodesJson
from src.util import get_logger

logger = get_logger(__name__)

app = typer.Typer()


def parse_keymap_c(keymap_path: Path, safe_range_start: int) -> CustomKeycodesJson:
    content = keymap_path.read_text()

    # Regex to find the enum custom_keycodes block
    pattern = re.compile(
        r"enum\s+custom_keycodes\s*\{([^}]+)\};", re.DOTALL | re.MULTILINE
    )
    match = pattern.search(content)

    if match is None:
        raise ValueError("enum custom_keycodes not found in keymap.c")

    enum_block = match.group(1)

    current_code: int = safe_range_start
    keycodes: dict[str, str] = {}

    # Remove comments
    enum_block = re.sub(r"//.*", "", enum_block)
    enum_block = re.sub(r"/\*.*?\*/", "", enum_block, flags=re.DOTALL)

    entries: list[str] = [e.strip() for e in enum_block.split(",") if e.strip()]

    for entry in entries:
        if "=" in entry:
            parts: list[str] = [x.strip() for x in entry.split("=", 1)]
            name = parts[0]
            value: str = parts[1]
            if value == "SAFE_RANGE":
                current_code = safe_range_start
        else:
            name = entry

        keycodes[f"0x{current_code:04X}"] = name
        current_code += 1

    return CustomKeycodesJson.model_validate(keycodes)


def get_safe_range_start(keycodes_json: Path) -> int:
    try:
        data = KeycodesJson.model_validate_json(keycodes_json.read_text())
    except Exception as e:
        raise OSError(f"Failed to load keycodes from {keycodes_json}: {e}") from e

    for code, name in data.root.items():
        if name == "SAFE_RANGE":
            return int(code, 16)
    raise ValueError(f"SAFE_RANGE not found in {keycodes_json}")


@app.command()
def main(
    keymap_c: Path = typer.Argument(..., help="Path to keymap.c"),
    custom_keycodes_json: Path = typer.Argument(
        ..., help="Path to output custom_keycodes.json"
    ),
    keycodes_json: Path = typer.Option(
        ..., "--keycodes-json", help="Path to keycodes.json to read SAFE_RANGE"
    ),
) -> None:
    """
    Sync custom keycodes from keymap.c to custom_keycodes.json
    """
    try:
        safe_range_start = get_safe_range_start(keycodes_json)

        custom_keycodes = parse_keymap_c(keymap_c, safe_range_start)
        custom_keycodes_json.write_text(
            custom_keycodes.model_dump_json(indent=4) + "\n"
        )
        logger.info(
            f"Successfully generated {custom_keycodes_json} with {len(custom_keycodes.root)} keycodes."
        )
    except Exception:
        logger.exception("Failed to sync custom keycodes from %s", keymap_c)
        raise typer.Exit(code=1) from None


if __name__ == "__main__":
    app()

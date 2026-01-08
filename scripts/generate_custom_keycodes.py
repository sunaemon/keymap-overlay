#!/usr/bin/env python3
# Copyright 2025 sunaemon
# SPDX-License-Identifier: MIT
import re
from pathlib import Path
from typing import Annotated

import typer

from src.types import KeycodesJson, parse_json, print_json
from src.util import get_logger, strip_c_comments

logger = get_logger(__name__)

app = typer.Typer()


def _parse_keymap_c(keymap_path: Path, safe_range_start: int) -> KeycodesJson:
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

    enum_block = strip_c_comments(enum_block)

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

    return KeycodesJson.model_validate(keycodes)


def _get_safe_range_start(keycodes_json: Path) -> int:
    keycodes_data = parse_json(KeycodesJson, keycodes_json)

    for code, name in keycodes_data.root.items():
        if name == "SAFE_RANGE":
            return int(code, 16)
    raise ValueError(f"SAFE_RANGE not found in {keycodes_json}")


def generate_custom_keycodes(keymap_c: Path, keycodes_json: Path) -> KeycodesJson:
    """Generate custom keycodes JSON from keymap.c and keycodes.json."""
    safe_range_start = _get_safe_range_start(keycodes_json)
    return _parse_keymap_c(keymap_c, safe_range_start)


@app.command()
def main(
    keymap_c: Annotated[Path, typer.Argument(help="Path to keymap.c")],
    keycodes_json: Annotated[
        Path,
        typer.Option(help="Path to keycodes.json to read SAFE_RANGE"),
    ],
) -> None:
    """
    Sync custom keycodes from keymap.c and emit JSON to stdout.
    """
    try:
        custom_keycodes = generate_custom_keycodes(keymap_c, keycodes_json)
        print_json(custom_keycodes)
        logger.info("Generated %d custom keycodes.", len(custom_keycodes.root))
    except Exception:
        logger.exception("Failed to sync custom keycodes from %s", keymap_c)
        raise typer.Exit(code=1) from None


if __name__ == "__main__":
    app()

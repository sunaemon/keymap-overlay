# Copyright 2025 sunaemon
# SPDX-License-Identifier: MIT
import logging
from pathlib import Path
from typing import Annotated

import typer

from model.src.types import KeycodesJson, VialJson, parse_json, print_json
from model.src.util import (
    CUSTOM_KEYCODE_BASE_NAMES,
    initialize_logging,
    parse_custom_keycode_names,
    parse_hex_keycode,
)

logger = logging.getLogger(__name__)

app = typer.Typer()

# Vial requires custom keycodes to be assigned starting at QK_KB_0; a device's
# embedded definition carries no numeric base of its own to look up.
VIAL_CUSTOM_KEYCODE_BASE = 0x7E00


@app.command()
def main(
    keymap_c: Annotated[Path | None, typer.Option(help="Path to keymap.c")] = None,
    keycodes_json: Annotated[
        Path | None,
        typer.Option(help="Path to keycodes.json to read SAFE_RANGE/QK_KB_0"),
    ] = None,
    vial_definition_json: Annotated[
        Path | None,
        typer.Option(help="Device-fetched Vial definition containing customKeycodes"),
    ] = None,
) -> None:
    """Sync custom keycodes from keymap.c or a device Vial definition."""
    initialize_logging()
    try:
        custom_keycodes = generate_custom_keycodes(
            keymap_c=keymap_c,
            keycodes_json=keycodes_json,
            vial_definition_json=vial_definition_json,
        )
        print_json(custom_keycodes)
        logger.info("Generated %d custom keycodes.", len(custom_keycodes.root))
    except Exception:
        logger.exception("Failed to generate custom keycodes")
        raise typer.Exit(code=1) from None


def generate_custom_keycodes(
    *,
    keymap_c: Path | None = None,
    keycodes_json: Path | None = None,
    vial_definition_json: Path | None = None,
) -> KeycodesJson:
    """Generate custom keycodes JSON from keymap.c or a device Vial definition."""
    if vial_definition_json is not None:
        return _from_vial_definition(vial_definition_json)
    if keymap_c is None or keycodes_json is None:
        raise ValueError("Provide keymap_c and keycodes_json, or vial_definition_json")
    base = _get_custom_keycode_base(keycodes_json)
    names = parse_custom_keycode_names(keymap_c)
    return KeycodesJson.model_validate(
        {f"0x{base + i:04X}": name for i, name in enumerate(names)}
    )


def _from_vial_definition(vial_definition_json: Path) -> KeycodesJson:
    definition = parse_json(VialJson, vial_definition_json)
    return KeycodesJson.model_validate(
        {
            f"0x{VIAL_CUSTOM_KEYCODE_BASE + i:04X}": keycode.name
            for i, keycode in enumerate(definition.customKeycodes or [])
        }
    )


def _get_custom_keycode_base(keycodes_json: Path) -> int:
    keycodes_data = parse_json(KeycodesJson, keycodes_json)

    for code, name in keycodes_data.root.items():
        if name in CUSTOM_KEYCODE_BASE_NAMES:
            parsed = parse_hex_keycode(code)
            if parsed is None:
                raise ValueError(f"Invalid {name} keycode: {code}")
            return parsed
    raise ValueError(
        f"None of {sorted(CUSTOM_KEYCODE_BASE_NAMES)} found in {keycodes_json}"
    )


if __name__ == "__main__":
    app()

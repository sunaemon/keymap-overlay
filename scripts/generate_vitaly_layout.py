#!/usr/bin/env python3
# Copyright 2025 sunaemon
# SPDX-License-Identifier: MIT
from pathlib import Path
from typing import Annotated

import typer

from src.types import (
    CustomKeycodesJson,
    KeyboardJson,
    QmkKeymapJson,
    VitalyJson,
    parse_json,
    print_json,
)
from src.util import get_logger

logger = get_logger(__name__)

app = typer.Typer()


def get_layout_mapping(
    keyboard_data: KeyboardJson,
    layout_name: str,
) -> list[tuple[int, int]]:
    """
    Returns a list where index i corresponds to the (row, col) of the i-th key
    in the flattened LAYOUT.
    """

    layouts = keyboard_data.layouts
    if layout_name not in layouts:
        raise ValueError(f"Layout {layout_name} not found in keyboard.json")

    layout_keys = layouts[layout_name].layout

    mapping: list[tuple[int, int]] = []
    for key in layout_keys:
        if key.matrix:
            mapping.append(key.matrix)
    return mapping


def generate_vitaly_layout(
    qmk_keymap_json: Path,
    vitaly_json: Path,
    keyboard_json: Path,
    custom_keycodes_json: Path,
    layout_name: str,
) -> VitalyJson:
    qmk_keymap_data = parse_json(QmkKeymapJson, qmk_keymap_json)
    vitaly_data = parse_json(VitalyJson, vitaly_json)
    keyboard_data = parse_json(KeyboardJson, keyboard_json)
    custom_keycodes_data = parse_json(CustomKeycodesJson, custom_keycodes_json)

    custom_map = {v: k for k, v in custom_keycodes_data.root.items()}

    mapping = get_layout_mapping(keyboard_data, layout_name)

    max_row = 0
    max_col = 0
    for r, c in mapping:
        max_row = max(max_row, r)
        max_col = max(max_col, c)

    rows = max_row + 1
    cols = max_col + 1

    qmk_layers = qmk_keymap_data.layers or []

    new_vitaly_layout = []

    for layer_idx, flat_layer in enumerate(qmk_layers):
        layer_grid: list[list[str | int]] = [
            ["KC_NO" for _ in range(cols)] for _ in range(rows)
        ]

        for key_idx, keycode in enumerate(flat_layer):
            if key_idx >= len(mapping):
                logger.warning(
                    "Layer %d has more keys than the layout definition", layer_idx
                )
                continue

            r, c = mapping[key_idx]

            if isinstance(keycode, str) and keycode in custom_map:
                keycode = custom_map[keycode]

            layer_grid[r][c] = keycode

        new_vitaly_layout.append(layer_grid)

    vitaly_data.layout = new_vitaly_layout
    return vitaly_data


@app.command()
def main(
    qmk_keymap_json: Annotated[Path, typer.Option(help="Source QMK keymap JSON")],
    vitaly_json: Annotated[Path, typer.Option(help="Base Vitaly JSON (to be updated)")],
    keyboard_json: Annotated[
        Path, typer.Option(help="QMK Keyboard JSON (for matrix mapping)")
    ],
    custom_keycodes_json: Annotated[
        Path,
        typer.Option(help="Path to custom-keycodes.json for reverse mapping"),
    ],
    layout_name: Annotated[str, typer.Option(help="Layout name in keyboard.json")],
) -> None:
    """
    Update Vitaly JSON layout from QMK JSON and emit it to stdout.
    """
    try:
        vitaly_data = generate_vitaly_layout(
            qmk_keymap_json,
            vitaly_json,
            keyboard_json,
            custom_keycodes_json,
            layout_name,
        )
        print_json(vitaly_data)
        logger.info("Generated updated Vitaly layout.")
    except Exception:
        logger.exception("Failed to generate Vitaly layout JSON")
        raise typer.Exit(code=1) from None


if __name__ == "__main__":
    app()

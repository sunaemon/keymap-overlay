#!/usr/bin/env python3
# Copyright 2025 sunaemon
# SPDX-License-Identifier: MIT
from pathlib import Path
from typing import Annotated

import typer

from src.types import (
    KeyboardJson,
    KeycodesJson,
    QmkKeymapJson,
    VitalyJson,
    parse_json,
    print_json,
)
from src.util import get_logger

logger = get_logger(__name__)

app = typer.Typer()


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


def generate_vitaly_layout(
    qmk_keymap_json: Path,
    vitaly_json: Path,
    keyboard_json: Path,
    custom_keycodes_json: Path,
    layout_name: str,
) -> VitalyJson:
    """Update Vitaly layout data from a QMK keymap JSON."""
    qmk_keymap_data = parse_json(QmkKeymapJson, qmk_keymap_json)
    vitaly_data = parse_json(VitalyJson, vitaly_json)
    keyboard_data = parse_json(KeyboardJson, keyboard_json)
    custom_keycodes_data = parse_json(KeycodesJson, custom_keycodes_json)

    custom_map: dict[str, str] = {}
    for code, name in custom_keycodes_data.root.items():
        if name in custom_map:
            logger.warning(
                "Custom keycode %s already mapped to %s; overwriting with %s",
                name,
                custom_map[name],
                code,
            )
        custom_map[name] = code

    mapping, rows, cols = keyboard_data.layout_mapping_dimensions(layout_name)

    qmk_layers = qmk_keymap_data.layers or []

    new_vitaly_layout = [
        _build_layer_grid(flat_layer, mapping, rows, cols, layer_idx, custom_map)
        for layer_idx, flat_layer in enumerate(qmk_layers)
    ]

    vitaly_data.layout = new_vitaly_layout
    return vitaly_data


def _build_layer_grid(
    flat_layer: list[str],
    mapping: list[tuple[int, int]],
    rows: int,
    cols: int,
    layer_idx: int,
    custom_map: dict[str, str],
) -> list[list[str]]:
    layer_grid = _init_layer_grid(rows, cols)

    for key_idx, keycode in enumerate(flat_layer):
        if key_idx >= len(mapping):
            logger.warning(
                "Layer %d has more keys than the layout definition", layer_idx
            )
            continue

        r, c = mapping[key_idx]
        layer_grid[r][c] = custom_map.get(keycode, keycode)

    return layer_grid


def _init_layer_grid(rows: int, cols: int) -> list[list[str]]:
    return [["KC_NO" for _ in range(cols)] for _ in range(rows)]


if __name__ == "__main__":
    app()

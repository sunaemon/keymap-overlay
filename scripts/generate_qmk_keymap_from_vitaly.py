#!/usr/bin/env python3
# Copyright 2025 sunaemon
# SPDX-License-Identifier: MIT
from pathlib import Path
from typing import Annotated

import typer

from src.types import (
    QmkKeymapJson,
    VitalyJson,
    parse_json,
    print_json,
)
from src.util import get_logger, load_layout_keys

logger = get_logger(__name__)

app = typer.Typer()


def flatten_layer(
    layer_data: list[list[str]],
    layout_map: dict[tuple[int, int], int],
) -> list[str]:
    """Flatten a matrix layer into a QMK list using the layout map."""
    if not layout_map:
        raise ValueError("Layout map is empty")
    max_flattened_idx = max(layout_map.values())
    flattened_layer = ["KC_NO" for _ in range(max_flattened_idx + 1)]
    for r, row in enumerate(layer_data):
        for c, key in enumerate(row):
            flattened_idx = layout_map.get((r, c))
            if flattened_idx is not None:
                flattened_layer[flattened_idx] = key
    return flattened_layer


def generate_qmk_keymap_from_vitaly(
    vitaly_json: Path,
    keyboard_json: Path,
    layout_name: str,
) -> QmkKeymapJson:
    """Convert a Vitaly JSON dump into a QMK keymap JSON."""
    layout_list = load_layout_keys(keyboard_json, layout_name)

    layout_map: dict[tuple[int, int], int] = {}
    for i, entry in enumerate(layout_list):
        row, col = entry.matrix
        if (row, col) in layout_map:
            logger.warning(
                f"Duplicate matrix position ({row}, {col}) at index {i} in layout '{layout_name}'"
            )
        layout_map[(row, col)] = i

    vitaly_data = parse_json(VitalyJson, vitaly_json)

    layers = [flatten_layer(layer, layout_map) for layer in vitaly_data.layout]
    return QmkKeymapJson(
        version=1,
        layers=layers,
        layout=layout_name,
    )


@app.command()
def main(
    vitaly_json: Annotated[Path, typer.Option(help="Path to Vitaly JSON dump")],
    keyboard_json: Annotated[
        Path,
        typer.Option(help="Path to keyboard.json to map matrix to flattened layout"),
    ],
    layout_name: Annotated[str, typer.Option(help="Layout name in keyboard.json")],
) -> None:
    """Generate QMK keymap JSON from a Vitaly dump and emit to stdout."""
    try:
        output = generate_qmk_keymap_from_vitaly(
            vitaly_json,
            keyboard_json,
            layout_name,
        )
        print_json(output)
    except Exception:
        logger.exception("Failed to generate QMK keymap from %s", vitaly_json)
        raise typer.Exit(code=1) from None


if __name__ == "__main__":
    app()

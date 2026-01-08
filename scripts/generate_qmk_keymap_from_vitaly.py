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

LAYOUT_NAME = "LAYOUT"


def load_keycodes(keycodes_path: Path) -> dict[int, str]:
    keycodes_data = parse_json(KeycodesJson, keycodes_path)

    return {int(k, 16): v for k, v in keycodes_data.root.items()}


# Returns the mapping from (row, col) to flattened index
def load_layout_map(keyboard_json: Path) -> dict[tuple[int, int], int]:
    keyboard_data = parse_json(KeyboardJson, keyboard_json)

    layouts = keyboard_data.layouts
    if layouts is None:
        raise ValueError("No layouts found in keyboard.json")

    if LAYOUT_NAME not in layouts:
        raise ValueError(f"Layout {LAYOUT_NAME} not found in keyboard.json")

    layout_list = layouts[LAYOUT_NAME].layout

    mapping: dict[tuple[int, int], int] = {}
    for i, entry in enumerate(layout_list):
        matrix = entry.matrix
        if matrix is None:
            continue
        row, col = matrix
        mapping[(row, col)] = i
    return mapping


def flatten_layer(
    layer_data: list[list[str]],
    layout_map: dict[tuple[int, int], int],
) -> list[str]:
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
) -> QmkKeymapJson:
    layout_map = load_layout_map(keyboard_json)
    vitaly_data = parse_json(VitalyJson, vitaly_json)

    layers = [flatten_layer(layer, layout_map) for layer in vitaly_data.layout]

    return QmkKeymapJson(
        version=1,
        layers=layers,
        layout="LAYOUT",
    )


@app.command()
def main(
    vitaly_json: Annotated[Path, typer.Option(help="Path to Vitaly JSON dump")],
    keyboard_json: Annotated[
        Path,
        typer.Option(help="Path to keyboard.json to map matrix to flattened layout"),
    ],
) -> None:
    try:
        output = generate_qmk_keymap_from_vitaly(
            vitaly_json,
            keyboard_json,
        )
        print_json(output)
    except Exception:
        logger.exception("Failed to generate QMK keymap from %s", vitaly_json)
        raise typer.Exit(code=1) from None


if __name__ == "__main__":
    app()

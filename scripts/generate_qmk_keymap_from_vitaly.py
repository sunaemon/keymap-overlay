#!/usr/bin/env python3
# Copyright 2025 sunaemon
# SPDX-License-Identifier: MIT
from pathlib import Path
from typing import Annotated

import typer
from pydantic import TypeAdapter

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

LayerKey = str | int
Layer = list[LayerKey]
LayerGrid = list[list[LayerKey]]
LayerListAdapter = TypeAdapter(list[Layer])
LayerGridListAdapter = TypeAdapter(list[LayerGrid])

LAYOUT_NAME = "LAYOUT"


def load_keycodes(keycodes_path: Path) -> dict[int, str]:
    keycodes_data = parse_json(KeycodesJson, keycodes_path)

    return {int(k, 16): v for k, v in keycodes_data.root.items()}


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
    layer_data: LayerGrid,
    layout_map: dict[tuple[int, int], int] | None,
) -> Layer:
    if layout_map is None:
        flattened: Layer = []
        for row in layer_data:
            flattened.extend(row)
        return flattened

    max_idx = max(layout_map.values())
    default_key: LayerKey = "KC_NO"
    flattened_layer = [default_key for _ in range(max_idx + 1)]
    for r, row in enumerate(layer_data):
        for c, key in enumerate(row):
            idx = layout_map.get((r, c))
            if idx is not None:
                flattened_layer[idx] = key
    return flattened_layer


def parse_layers(
    raw_layers: object,
    source: str,
    layout_map: dict[tuple[int, int], int] | None,
) -> list[Layer]:
    try:
        grid_layers: list[LayerGrid] = LayerGridListAdapter.validate_python(raw_layers)
    except Exception:
        try:
            flat_layers: list[Layer] = LayerListAdapter.validate_python(raw_layers)
            return flat_layers
        except Exception as exc:
            raise ValueError(f"Invalid {source} layers") from exc

    return [flatten_layer(layer, layout_map) for layer in grid_layers]


def generate_qmk_keymap_from_vitaly(
    vitaly_json: Path,
    keycodes_json: Path,
    keyboard_json: Path,
) -> QmkKeymapJson:
    keycodes_map = load_keycodes(keycodes_json)
    layout_map = load_layout_map(keyboard_json)
    vitaly_data = parse_json(VitalyJson, vitaly_json)

    if not vitaly_data.layout:
        raise ValueError("Vitaly JSON is missing layout data")

    layers = parse_layers(vitaly_data.layout, "vitaly.layout", layout_map)

    new_layers: list[Layer] = []
    for layer in layers:
        new_layer: Layer = []
        for key in layer:
            if isinstance(key, int):
                if key in keycodes_map:
                    new_layer.append(keycodes_map[key])
                else:
                    new_layer.append(f"0x{key:04X}")
            else:
                new_layer.append(str(key))
        new_layers.append(new_layer)

    return QmkKeymapJson(
        version=1,
        layers=new_layers,
        layout="LAYOUT",
    )


@app.command()
def main(
    vitaly_json: Annotated[Path, typer.Option(help="Path to Vitaly JSON dump")],
    keycodes_json: Annotated[Path, typer.Option(help="Path to keycodes.json map")],
    keyboard_json: Annotated[
        Path,
        typer.Option(help="Path to keyboard.json to map matrix to flattened layout"),
    ],
) -> None:
    try:
        output = generate_qmk_keymap_from_vitaly(
            vitaly_json,
            keycodes_json,
            keyboard_json,
        )
        print_json(output)
    except Exception:
        logger.exception("Failed to generate QMK keymap from %s", vitaly_json)
        raise typer.Exit(code=1) from None


if __name__ == "__main__":
    app()

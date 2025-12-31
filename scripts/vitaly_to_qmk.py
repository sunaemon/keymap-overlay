#!/usr/bin/env python3
# Copyright 2025 sunaemon
# SPDX-License-Identifier: MIT
import json
from pathlib import Path
from typing import Annotated

import typer
from pydantic import TypeAdapter

from src.types import (
    KeycodesJson,
    LayoutKey,
    QmkKeyboardJson,
    QmkKeymapJson,
    VitalyJson,
)
from src.util import get_logger

logger = get_logger(__name__)

app = typer.Typer()

LayerKey = str | int
Layer = list[LayerKey]
LayerGrid = list[list[LayerKey]]
LayerListAdapter = TypeAdapter(list[Layer])
LayerGridListAdapter = TypeAdapter(list[LayerGrid])


def load_keycodes(keycodes_path: Path) -> dict[int, str]:
    try:
        raw_data = json.loads(keycodes_path.read_text())
        keycodes = KeycodesJson.model_validate(raw_data)
        return {int(k, 16): v for k, v in keycodes.root.items()}
    except Exception as e:
        logger.warning(f"Warning: Failed to load keycodes from {keycodes_path}: {e}")
        return {}


def load_layout_map(keyboard_json: Path) -> dict[tuple[int, int], int] | None:
    try:
        raw_data = json.loads(keyboard_json.read_text())
        qmk_data = TypeAdapter(QmkKeyboardJson).validate_python(raw_data)

        if not qmk_data.layouts:
            return None

        layout_name = "LAYOUT"
        if layout_name not in qmk_data.layouts:
            layout_name = next(iter(qmk_data.layouts))

        layout_list: list[LayoutKey] = qmk_data.layouts[layout_name].layout

        mapping: dict[tuple[int, int], int] = {}
        for i, entry in enumerate(layout_list):
            matrix = entry.matrix
            if matrix is None:
                continue
            row, col = matrix
            mapping[(row, col)] = i
        return mapping
    except Exception as e:
        logger.warning(f"Warning: Failed to load layout map from {keyboard_json}: {e}")
        return None


def flatten_layer(
    layer_data: LayerGrid,
    layout_map: dict[tuple[int, int], int] | None,
) -> Layer:
    flattened_layer: Layer
    if not layout_map:
        flattened_layer = []
        for row in layer_data:
            flattened_layer.extend(row)
        return flattened_layer

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


@app.command()
def main(
    vitaly_json: Annotated[Path, typer.Option(help="Path to Vitaly JSON dump")],
    keycodes_json: Annotated[
        Path, typer.Option(help="Path to keycodes.json map")
    ] = Path("build/keycodes.json"),
    keyboard_json: Annotated[
        Path | None,
        typer.Option(help="Path to keyboard.json to map matrix to flattened layout"),
    ] = None,
) -> None:
    """
    Convert Vitaly JSON to QMK Keymap JSON
    """
    keycodes_map = load_keycodes(keycodes_json)
    layout_map: dict[tuple[int, int], int] | None = None
    if keyboard_json:
        layout_map = load_layout_map(keyboard_json)

    try:
        data = json.loads(vitaly_json.read_text())
    except Exception:
        logger.exception(f"Error reading {vitaly_json}")
        raise typer.Exit(code=1)

    layers: list[Layer] = []
    if isinstance(data, dict):
        try:
            vitaly_data = TypeAdapter(VitalyJson).validate_python(data)
        except Exception:
            logger.exception(f"Error parsing {vitaly_json}")
            raise typer.Exit(code=1)

        try:
            if vitaly_data.layers:
                layers = parse_layers(vitaly_data.layers, "vitaly.layers", layout_map)
            elif vitaly_data.layout:
                layers = parse_layers(vitaly_data.layout, "vitaly.layout", layout_map)
            else:
                logger.error("Error: Could not find layers in input JSON")
                raise typer.Exit(code=1)
        except ValueError:
            logger.exception("Error parsing layers from %s", vitaly_json)
            raise typer.Exit(code=1)
    elif isinstance(data, list) and data and isinstance(data[0], list):
        try:
            layers = parse_layers(data, "input", layout_map)
        except ValueError:
            logger.exception("Error parsing layers from %s", vitaly_json)
            raise typer.Exit(code=1)
    else:
        logger.error("Error: Could not find layers in input JSON")
        raise typer.Exit(code=1)

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

    output = QmkKeymapJson(
        version=1,
        layers=new_layers,
        layout="LAYOUT",
    )

    print(json.dumps(output.model_dump(mode="json"), indent=4))


if __name__ == "__main__":
    app()

#!/usr/bin/env python3
# Copyright 2025 sunaemon
# SPDX-License-Identifier: MIT
import json
from pathlib import Path
from typing import Annotated, Any

import typer
from pydantic import TypeAdapter

from src.types import (
    CustomKeycodesJson,
    QmkKeyboardJson,
    QmkKeymapJson,
    VitalyJson,
)
from src.util import get_logger

logger = get_logger(__name__)

app = typer.Typer()


def load_json(path: Path) -> Any:  # noqa: ANN401
    try:
        return json.loads(path.read_text())
    except Exception as exc:
        raise ValueError(f"Failed to load {path}") from exc


def get_layout_mapping(keyboard_data: QmkKeyboardJson) -> list[tuple[int, int]]:
    """
    Returns a list where index i corresponds to the (row, col) of the i-th key
    in the flattened LAYOUT.
    """

    layouts = keyboard_data.layouts
    layout_name = "LAYOUT"
    if layout_name not in layouts:
        layout_name = next(iter(layouts))

    layout_keys = layouts[layout_name].layout

    mapping: list[tuple[int, int]] = []
    for key in layout_keys:
        if key.matrix:
            mapping.append(key.matrix)
    return mapping


@app.command()
def main(
    qmk_keymap_json: Annotated[Path, typer.Option(help="Source QMK keymap JSON")],
    vitaly_json: Annotated[Path, typer.Option(help="Base Vitaly JSON (to be updated)")],
    keyboard_json: Annotated[
        Path, typer.Option(help="QMK Keyboard JSON (for matrix mapping)")
    ],
    output: Annotated[Path, typer.Option(help="Output Vitaly JSON")],
    custom_keycodes_json: Annotated[
        Path | None,
        typer.Option(help="Path to custom-keycodes.json for reverse mapping"),
    ] = None,
) -> None:
    """
    Update Vitaly JSON layout from QMK JSON
    """
    try:
        raw_qmk_data = load_json(qmk_keymap_json)
        qmk_data = TypeAdapter(QmkKeymapJson).validate_python(raw_qmk_data)
        raw_vitaly_data = load_json(vitaly_json)
        vitaly_data = TypeAdapter(VitalyJson).validate_python(raw_vitaly_data)
        raw_keyboard_data = load_json(keyboard_json)
        keyboard_data = TypeAdapter(QmkKeyboardJson).validate_python(raw_keyboard_data)
    except Exception:
        logger.exception("Failed to load input JSON")
        raise typer.Exit(code=1) from None

    custom_map = {}
    if custom_keycodes_json:
        try:
            raw_map = load_json(custom_keycodes_json)
            custom_keycodes = TypeAdapter(CustomKeycodesJson).validate_python(raw_map)
            for k, v in custom_keycodes.root.items():
                custom_map[v] = k
        except Exception:
            logger.warning(
                "Failed to load custom keycodes from %s", custom_keycodes_json
            )

    mapping = get_layout_mapping(keyboard_data)

    max_row = 0
    max_col = 0
    for r, c in mapping:
        max_row = max(max_row, r)
        max_col = max(max_col, c)

    rows = max_row + 1
    cols = max_col + 1

    qmk_layers = qmk_data.layers or []

    new_vitaly_layout = []

    for layer_idx, flat_layer in enumerate(qmk_layers):
        layer_grid: list[list[str | int]] = [
            ["KC_NO" for _ in range(cols)] for _ in range(rows)
        ]

        for key_idx, keycode in enumerate(flat_layer):
            if key_idx >= len(mapping):
                logger.warning(
                    f"Warning: Layer {layer_idx} has more keys than the layout definition"
                )
                continue

            r, c = mapping[key_idx]

            if isinstance(keycode, str) and keycode in custom_map:
                keycode = custom_map[keycode]

            layer_grid[r][c] = keycode

        new_vitaly_layout.append(layer_grid)

    vitaly_data.layout = new_vitaly_layout

    try:
        output.write_text(vitaly_data.model_dump_json(indent=None, exclude_none=True))
        logger.info(f"Successfully wrote updated layout to {output}")
    except OSError:
        logger.exception("Failed to write %s", output)
        raise typer.Exit(code=1) from None


if __name__ == "__main__":
    app()

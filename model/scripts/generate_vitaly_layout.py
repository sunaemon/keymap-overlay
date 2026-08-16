# Copyright 2025 sunaemon
# SPDX-License-Identifier: MIT
import logging
from pathlib import Path
from typing import Annotated

import typer

from model.scripts.encoder_map import parse_encoder_map
from model.src.types import (
    KeyboardJson,
    KeycodesJson,
    QmkKeymapJson,
    VitalyJson,
    parse_json,
    print_json,
)
from model.src.util import initialize_logging

logger = logging.getLogger(__name__)

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
    keymap_c: Annotated[Path, typer.Option(help="keymap.c containing encoder_map")],
    layout_name: Annotated[str, typer.Option(help="Layout name in keyboard.json")],
) -> None:
    """Update Vitaly JSON layout from QMK JSON and emit it to stdout."""
    initialize_logging()
    try:
        vitaly_data = generate_vitaly_layout(
            qmk_keymap_json,
            vitaly_json,
            keyboard_json,
            custom_keycodes_json,
            keymap_c,
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
    keymap_c: Path,
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
    encoder_layers = parse_encoder_map(keymap_c)
    encoder_count = keyboard_data.encoder_count()
    if encoder_layers or encoder_count:
        vitaly_data.encoder_layout = _build_encoder_layout(
            encoder_layers,
            encoder_count,
            len(qmk_layers),
            custom_map,
        )
    return vitaly_data


def _build_layer_grid(
    flat_layer: list[str],
    mapping: list[tuple[int, int]],
    rows: int,
    cols: int,
    layer_idx: int,
    custom_map: dict[str, str],
) -> list[list[str]]:
    """Place one flat QMK layer into its matrix-shaped VIAL grid."""
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
    """Create an empty VIAL layer grid with the requested dimensions."""
    return [["KC_NO" for _ in range(cols)] for _ in range(rows)]


def _build_encoder_layout(
    encoder_layers: list[list[list[str]]],
    encoder_count: int,
    layer_count: int,
    custom_map: dict[str, str],
) -> list[list[list[str]]]:
    """Build one padded VIAL encoder-action list per keymap layer."""
    output: list[list[list[str]]] = []
    for layer_index in range(layer_count):
        pairs = encoder_layers[layer_index] if layer_index < len(encoder_layers) else []
        if len(pairs) > encoder_count:
            raise ValueError(
                f"Layer {layer_index} defines {len(pairs)} encoders, expected at most {encoder_count}"
            )
        if any(len(pair) != 2 for pair in pairs):
            raise ValueError(
                f"Layer {layer_index} encoder bindings must have two directions"
            )
        converted = [
            [custom_map.get(keycode, keycode) for keycode in pair] for pair in pairs
        ]
        converted.extend(
            [["KC_NO", "KC_NO"] for _ in range(encoder_count - len(converted))]
        )
        output.append(converted)
    return output


if __name__ == "__main__":
    app()

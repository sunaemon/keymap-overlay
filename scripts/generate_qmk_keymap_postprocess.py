#!/usr/bin/env python3
# Copyright 2025 sunaemon
# SPDX-License-Identifier: MIT
from pathlib import Path
from typing import Annotated

import typer

from src.types import CustomKeycodesJson, QmkKeymapJson, parse_json, print_json
from src.util import get_logger

logger = get_logger(__name__)

app = typer.Typer()


def _resolve_transparency(keymap: QmkKeymapJson) -> QmkKeymapJson:
    if not keymap.layers:
        return keymap

    layers = keymap.layers
    TRANS_KEYS = {"KC_TRNS", "KC_TRANSPARENT", "_______"}

    for i in range(1, len(layers)):
        for idx in range(len(layers[i])):
            if layers[i][idx] in TRANS_KEYS:
                for j in range(i - 1, -1, -1):
                    if idx < len(layers[j]):
                        prev_key = layers[j][idx]
                        if prev_key not in TRANS_KEYS:
                            layers[i][idx] = prev_key
                            break
    return keymap


def _load_custom_keycodes(custom_keycodes_path: Path) -> dict[int, str] | None:
    try:
        custom_keycodes_data = parse_json(CustomKeycodesJson, custom_keycodes_path)
    except Exception as e:
        logger.warning(
            "Could not load custom keycodes from %s: %s", custom_keycodes_path, e
        )
        return None

    int_map: dict[int, str] = {}
    for k, v in custom_keycodes_data.root.items():
        try:
            int_map[int(k, 16)] = v
        except ValueError:
            logger.warning("Skipping non-hex custom keycode entry: %s", k)
            continue
    return int_map


def _parse_keycode_value(key: str | int) -> int | None:
    if isinstance(key, int):
        return key
    if key.startswith(("0x", "0X")):
        try:
            return int(key, 16)
        except ValueError:
            return None
    if key.isdigit():
        try:
            return int(key)
        except ValueError:
            return None
    return None


def _apply_custom_map_to_layer(layer: list[str], int_map: dict[int, str]) -> None:
    for idx, key in enumerate(layer):
        code_val = _parse_keycode_value(key)
        if code_val is not None and code_val in int_map:
            layer[idx] = int_map[code_val]


def _apply_custom_keycodes(
    keymap: QmkKeymapJson, custom_keycodes_path: Path
) -> QmkKeymapJson:
    int_map = _load_custom_keycodes(custom_keycodes_path)
    if not int_map or not keymap.layers:
        return keymap

    for layer in keymap.layers:
        _apply_custom_map_to_layer(layer, int_map)

    return keymap


def _process_keymap(qmk_keymap_json: Path, custom_keycodes_json: Path) -> QmkKeymapJson:
    keymap_data = parse_json(QmkKeymapJson, qmk_keymap_json)
    keymap_data = _apply_custom_keycodes(keymap_data, custom_keycodes_json)
    return _resolve_transparency(keymap_data)


@app.command()
def main(
    qmk_keymap_json: Annotated[
        Path, typer.Argument(help="Path to the QMK keymap JSON file")
    ],
    custom_keycodes_json: Annotated[
        Path, typer.Option(help="Path to custom_keycodes.json")
    ],
) -> None:
    """
    Process QMK keymap JSON and emit the result to stdout.
    """
    try:
        resolved_keymap = _process_keymap(qmk_keymap_json, custom_keycodes_json)
        print_json(resolved_keymap)
        logger.info("Processed QMK keymap JSON from %s", qmk_keymap_json)

    except Exception:
        logger.exception("Failed to process %s", qmk_keymap_json)
        raise typer.Exit(code=1) from None


if __name__ == "__main__":
    app()

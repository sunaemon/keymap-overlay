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


def resolve_transparency(keymap: QmkKeymapJson) -> QmkKeymapJson:
    if not keymap.layers:
        return keymap

    layers = keymap.layers
    TRANS_KEYS = {"KC_TRNS", "KC_TRANSPARENT", "_______", "KC_TRANS"}

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


def apply_custom_keycodes(
    keymap: QmkKeymapJson, custom_keycodes_path: Path
) -> QmkKeymapJson:
    try:
        custom_keycodes_data = parse_json(CustomKeycodesJson, custom_keycodes_path)
        custom_map = custom_keycodes_data.root
    except Exception as e:
        logger.warning(
            "Could not load custom keycodes from %s: %s", custom_keycodes_path, e
        )
        return keymap

    # Convert generated "0xABCD" keys to integer for easier matching
    int_map: dict[int, str] = {}
    for k, v in custom_map.items():
        try:
            code = int(k, 16)
            int_map[code] = v
        except ValueError:
            continue

    if not keymap.layers:
        return keymap

    for layer in keymap.layers:
        for idx, key in enumerate(layer):
            code_val: int | None = None
            if isinstance(key, int):
                code_val = key
            elif isinstance(key, str):
                if key.startswith("0x") or key.startswith("0X"):
                    try:
                        code_val = int(key, 16)
                    except ValueError:
                        pass
                elif key.isdigit():
                    try:
                        code_val = int(key)
                    except ValueError:
                        pass

            if code_val is not None and code_val in int_map:
                layer[idx] = int_map[code_val]

    return keymap


def process_keymap(qmk_keymap_json: Path, custom_keycodes_json: Path) -> QmkKeymapJson:
    keymap_data = parse_json(QmkKeymapJson, qmk_keymap_json)
    keymap_data = apply_custom_keycodes(keymap_data, custom_keycodes_json)
    return resolve_transparency(keymap_data)


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
        resolved_keymap = process_keymap(qmk_keymap_json, custom_keycodes_json)
        print_json(resolved_keymap)

    except Exception:
        logger.exception("Failed to process %s", qmk_keymap_json)
        raise typer.Exit(code=1) from None


if __name__ == "__main__":
    app()

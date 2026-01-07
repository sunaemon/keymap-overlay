#!/usr/bin/env python3
# Copyright 2025 sunaemon
# SPDX-License-Identifier: MIT
from collections import defaultdict
from pathlib import Path
from typing import Annotated

import typer

from src.types import (
    KeyboardJson,
    VialJson,
    VialLayouts,
    VialMatrix,
    parse_json,
    print_json,
)
from src.util import get_logger

logger = get_logger(__name__)

app = typer.Typer()


def generate_vial(keyboard_json: Path) -> VialJson:
    keyboard_data = parse_json(KeyboardJson, keyboard_json)

    # 1. Extract Metadata
    vendor_id = "0x0000"
    product_id = "0x0000"

    if keyboard_data.usb:
        vendor_id = keyboard_data.usb.vid
        product_id = keyboard_data.usb.pid

    # 2. Extract Matrix Info
    rows_count = 0
    cols_count = 0

    if keyboard_data.matrix_pins:
        if keyboard_data.matrix_pins.rows:
            rows_count = len(keyboard_data.matrix_pins.rows)
        if keyboard_data.matrix_pins.cols:
            cols_count = len(keyboard_data.matrix_pins.cols)

    # 3. Process Layout
    layout_name = "LAYOUT"
    if layout_name not in keyboard_data.layouts:
        # Fallback to first layout
        if keyboard_data.layouts:
            layout_name = next(iter(keyboard_data.layouts))
        else:
            logger.critical("No layouts found in %s", keyboard_json)
            raise typer.Exit(code=1)

    layout_data = keyboard_data.layouts[layout_name].layout

    # Determine max rows/cols from layout data if not set
    max_row = 0
    max_col = 0
    for key in layout_data:
        if key.matrix:
            r, c = key.matrix
            if r > max_row:
                max_row = r
            if c > max_col:
                max_col = c

    if rows_count == 0:
        rows_count = max_row + 1
    if cols_count == 0:
        cols_count = max_col + 1

    # Update matrix info in vial_data
    matrix = VialMatrix(
        rows=max(rows_count, max_row + 1),
        cols=max(cols_count, max_col + 1),
    )

    # 4. Convert to KLE Format
    rows = defaultdict(list)
    for key in layout_data:
        rows[key.y].append(key)

    sorted_y = sorted(rows.keys())

    kle_rows = []

    for y in sorted_y:
        row_keys = sorted(rows[y], key=lambda k: k.x)
        kle_row: list[str | dict[str, float]] = []
        current_x = 0.0

        for key in row_keys:
            key_x = key.x
            key_w = key.w
            key_h = key.h

            props = {}

            # X spacing
            x_diff = key_x - current_x
            if abs(x_diff) > 0.01:
                props["x"] = round(x_diff, 2)

            # Width
            if abs(key_w - 1) > 0.01:
                props["w"] = round(key_w, 2)

            # Height
            if abs(key_h - 1) > 0.01:
                props["h"] = round(key_h, 2)

            # Add props if any
            if props:
                kle_row.append(props)

            # Add label
            if key.matrix:
                r, c = key.matrix
                kle_row.append(f"{r},{c}")
            else:
                kle_row.append("-1,-1")

            current_x = key_x + key_w

        kle_rows.append(kle_row)

    return VialJson(
        name=keyboard_data.keyboard_name,
        vendorId=vendor_id,
        productId=product_id,
        matrix=matrix,
        layouts=VialLayouts(keymap=kle_rows),
    )


@app.command()
def main(
    keyboard_json: Annotated[Path, typer.Option(help="Path to input keyboard.json")],
) -> None:
    """
    Convert QMK info.json (keyboard.json) to Vial JSON and emit it to stdout.
    """
    try:
        vial_data = generate_vial(keyboard_json)
        print_json(vial_data)
        logger.info("Generated Vial JSON from %s", keyboard_json)
    except Exception:
        logger.exception("Failed to generate Vial JSON from %s", keyboard_json)
        raise typer.Exit(code=1) from None


if __name__ == "__main__":
    app()

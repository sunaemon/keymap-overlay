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

    if not keyboard_data.usb:
        raise ValueError("No USB info found in keyboard.json")
    vendor_id = keyboard_data.usb.vid
    product_id = keyboard_data.usb.pid

    rows_count = 0
    cols_count = 0

    if not keyboard_data.matrix_pins:
        raise ValueError("No matrix pins info found in keyboard.json")

    rows_count = len(keyboard_data.matrix_pins.rows)
    cols_count = len(keyboard_data.matrix_pins.cols)

    if keyboard_data.split and keyboard_data.split.enabled:
        if len(keyboard_data.split.matrix_pins) != 1:
            raise ValueError("multiple split sides not supported yet")
        split_side, matrix_pins = next(iter(keyboard_data.split.matrix_pins.items()))
        if split_side != "left" and split_side != "right":
            raise ValueError(
                "only left and right side split configurations are supported yet"
            )
        rows_count += len(matrix_pins.rows)

    layout_name = "LAYOUT"
    if layout_name not in keyboard_data.layouts:
        raise ValueError(f"Layout {layout_name} not found in keyboard.json")

    layout_data = keyboard_data.layouts[layout_name].layout

    for key in layout_data:
        if key.matrix:
            r, c = key.matrix
            if r > rows_count:
                raise ValueError("Matrix rows count is inconsistent with layout data")
            if c > cols_count:
                raise ValueError(
                    "Matrix columns count is inconsistent with layout data"
                )

    matrix = VialMatrix(
        rows=rows_count,
        cols=cols_count,
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

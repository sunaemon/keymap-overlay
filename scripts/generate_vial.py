# Copyright 2025 sunaemon
# SPDX-License-Identifier: MIT
import logging
from pathlib import Path
from typing import Annotated

import typer

from src.types import (
    KeyboardJson,
    KleKeyProps,
    KleLayout,
    KleRow,
    LayoutKey,
    VialJson,
    VialLayouts,
    VialMatrix,
    parse_json,
    print_json,
)
from src.util import initialize_logging

logger = logging.getLogger(__name__)

app = typer.Typer()

PRECISION = 2  # We handle key positions with 1<<2 = 4 subdivisions per unit


@app.command()
def main(
    keyboard_json: Annotated[Path, typer.Option(help="Path to input keyboard.json")],
    layout_name: Annotated[str, typer.Option(help="Layout name in keyboard.json")],
) -> None:
    """Convert QMK info.json (keyboard.json) to Vial JSON and emit it to stdout."""
    initialize_logging()
    try:
        vial_data = generate_vial(keyboard_json, layout_name)
        print_json(vial_data, exclude_none=True)
        logger.info("Generated Vial JSON from %s", keyboard_json)
    except Exception:
        logger.exception("Failed to generate Vial JSON from %s", keyboard_json)
        raise typer.Exit(code=1) from None


def generate_vial(keyboard_json: Path, layout_name: str) -> VialJson:
    """Convert QMK keyboard.json to a Vial-compatible JSON structure."""
    keyboard_data = parse_json(KeyboardJson, keyboard_json)

    vendor_id = keyboard_data.usb.vid
    product_id = keyboard_data.usb.pid

    matrix_rows, matrix_cols = keyboard_data.matrix_dimensions()

    layout_data = keyboard_data.layout_keys(layout_name)
    rows_by_y = _group_layout_rows(layout_data)
    kle_rows = _build_kle_rows(rows_by_y)

    return VialJson(
        name=keyboard_data.keyboard_name,
        vendorId=vendor_id,
        productId=product_id,
        matrix=VialMatrix(rows=matrix_rows, cols=matrix_cols),
        layouts=VialLayouts(keymap=kle_rows),
    )


def _group_layout_rows(layout_data: list[LayoutKey]) -> dict[float, list[LayoutKey]]:
    rows: dict[float, list[LayoutKey]] = {}
    for key in layout_data:
        row_index = _round_unit(key.y)
        rows.setdefault(row_index, []).append(key)
    return rows


def _build_kle_rows(rows_by_y: dict[float, list[LayoutKey]]) -> KleLayout:
    kle_rows: KleLayout = []

    current_cursor_y = 0.0

    for y in sorted(rows_by_y.keys()):
        row_keys = rows_by_y[y]
        kle_row = _build_kle_row(row_keys)

        required_y = _round_unit(y)
        y_diff = _round_unit(required_y - current_cursor_y)

        if y_diff != 0:
            first_item = kle_row[0]
            if isinstance(first_item, KleKeyProps):
                first_item.y = (first_item.y or 0) + y_diff
            else:
                new_props = KleKeyProps()
                new_props.y = y_diff
                kle_row.insert(0, new_props)

        kle_rows.append(kle_row)

        current_cursor_y = required_y + 1.0

    return kle_rows


def _build_kle_row(row_keys: list[LayoutKey]) -> KleRow:
    row_keys = sorted(row_keys, key=lambda k: k.x)
    kle_row: KleRow = []
    current_x = 0.0

    for key in row_keys:
        key_x = _round_unit(key.x)
        key_w = _round_unit(key.w)
        key_h = _round_unit(key.h)

        props = KleKeyProps()

        if key_x != current_x:
            props.x = key_x - current_x

        if key_w != 1:
            props.w = key_w

        if key_h != 1:
            props.h = key_h

        if props.has_values():
            kle_row.append(props)

        r, c = key.matrix
        kle_row.append(f"{r},{c}")

        current_x = key_x + key_w

    return kle_row


def _round_unit(x: float) -> float:
    return round(x * (1 << PRECISION)) / (1 << PRECISION)


if __name__ == "__main__":
    app()

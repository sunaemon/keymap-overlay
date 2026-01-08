#!/usr/bin/env python3
# Copyright 2025 sunaemon
# SPDX-License-Identifier: MIT
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
from src.util import get_logger

logger = get_logger(__name__)

app = typer.Typer()


PRECISION = 2  # We handle key positions with 1<<2 = 4 subdivisions per unit


def _round_unit(x: float) -> float:
    return round(x * (1 << PRECISION)) / (1 << PRECISION)


def _get_matrix_rows(keyboard_data: KeyboardJson) -> int:
    rows = len(keyboard_data.matrix_pins.rows)
    if keyboard_data.split and keyboard_data.split.enabled:
        if len(keyboard_data.split.matrix_pins) != 1:
            raise ValueError("multiple split sides not supported yet")
        split_side, matrix_pins = next(iter(keyboard_data.split.matrix_pins.items()))
        if split_side != "left" and split_side != "right":
            raise ValueError(
                "only left and right side split configurations are supported yet"
            )
        rows += len(matrix_pins.rows)
    return rows


def _get_layout_data(
    keyboard_data: KeyboardJson,
    layout_name: str,
) -> list[LayoutKey]:
    if layout_name not in keyboard_data.layouts:
        raise ValueError(f"Layout {layout_name} not found in keyboard.json")
    return keyboard_data.layouts[layout_name].layout


def _validate_layout_matrix(
    layout_data: list[LayoutKey],
    matrix_rows: int,
    matrix_cols: int,
) -> None:
    for key in layout_data:
        r, c = key.matrix
        if r >= matrix_rows:
            raise ValueError("Matrix rows count is inconsistent with layout data")
        if c >= matrix_cols:
            raise ValueError("Matrix columns count is inconsistent with layout data")


def _group_layout_rows(layout_data: list[LayoutKey]) -> dict[int, list[LayoutKey]]:
    rows: dict[int, list[LayoutKey]] = {}
    for key in layout_data:
        row_index = int(key.y)
        if row_index != _round_unit(key.y):
            raise ValueError("Non-integer key y position is not supported")
        rows.setdefault(row_index, []).append(key)
    return rows


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


def _build_kle_rows(rows_by_y: dict[int, list[LayoutKey]]) -> KleLayout:
    kle_rows: KleLayout = []
    for y in sorted(rows_by_y.keys()):
        kle_rows.append(_build_kle_row(rows_by_y[y]))
    return kle_rows


def generate_vial(keyboard_json: Path, layout_name: str) -> VialJson:
    """Convert QMK keyboard.json to a Vial-compatible JSON structure."""
    keyboard_data = parse_json(KeyboardJson, keyboard_json)

    vendor_id = keyboard_data.usb.vid
    product_id = keyboard_data.usb.pid

    matrix_rows = _get_matrix_rows(keyboard_data)
    matrix_cols = len(keyboard_data.matrix_pins.cols)

    layout_data = _get_layout_data(keyboard_data, layout_name)
    _validate_layout_matrix(layout_data, matrix_rows, matrix_cols)

    rows_by_y = _group_layout_rows(layout_data)
    kle_rows = _build_kle_rows(rows_by_y)

    return VialJson(
        name=keyboard_data.keyboard_name,
        vendorId=vendor_id,
        productId=product_id,
        matrix=VialMatrix(rows=matrix_rows, cols=matrix_cols),
        layouts=VialLayouts(keymap=kle_rows),
    )


@app.command()
def main(
    keyboard_json: Annotated[Path, typer.Option(help="Path to input keyboard.json")],
    layout_name: Annotated[str, typer.Option(help="Layout name in keyboard.json")],
) -> None:
    """
    Convert QMK info.json (keyboard.json) to Vial JSON and emit it to stdout.
    """
    try:
        vial_data = generate_vial(keyboard_json, layout_name)
        print_json(vial_data, exclude_none=True)
        logger.info("Generated Vial JSON from %s", keyboard_json)
    except Exception:
        logger.exception("Failed to generate Vial JSON from %s", keyboard_json)
        raise typer.Exit(code=1) from None


if __name__ == "__main__":
    app()

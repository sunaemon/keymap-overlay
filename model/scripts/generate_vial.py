# Copyright 2025 sunaemon
# SPDX-License-Identifier: MIT
import logging
from pathlib import Path
from typing import Annotated

import typer

from model.src.types import (
    KeyboardConfig,
    KeyboardJson,
    KeymapOverlayMetadata,
    KleKeyProps,
    KleLayout,
    KleRow,
    LayoutKey,
    VialCustomKeycode,
    VialJson,
    VialLayouts,
    VialMatrix,
    parse_json,
    print_json,
)
from model.src.util import (
    initialize_logging,
    parse_custom_keycode_names,
    parse_custom_keycode_short_names,
)

logger = logging.getLogger(__name__)

app = typer.Typer()

PRECISION = 2  # We handle key positions with 1<<2 = 4 subdivisions per unit
ENCODER_PAIR_GAP = 0.25
VIAL_ENCODER_LEGEND_SUFFIX = "\n\n\n\n\n\n\n\n\ne"


@app.command()
def main(
    keyboard_json: Annotated[Path, typer.Option(help="Path to input keyboard.json")],
    layout_name: Annotated[str, typer.Option(help="Layout name in keyboard.json")],
    keymap_c: Annotated[
        Path | None,
        typer.Option(help="keymap.c containing enum custom_keycodes"),
    ] = None,
    keyboard_config: Annotated[
        Path | None,
        typer.Option(help="Project config to embed for keymap-overlay"),
    ] = None,
    keyboard_id: Annotated[
        int | None,
        typer.Option(help="KMO keyboard ID to embed for keymap-overlay"),
    ] = None,
    pixels_per_unit: Annotated[int, typer.Option()] = 64,
) -> None:
    """Convert QMK info.json (keyboard.json) to Vial JSON and emit it to stdout."""
    initialize_logging()
    try:
        vial_data = generate_vial(
            keyboard_json,
            layout_name,
            keymap_c=keymap_c,
            keyboard_config=keyboard_config,
            keyboard_id=keyboard_id,
            pixels_per_unit=pixels_per_unit,
        )
        print_json(vial_data, exclude_none=True)
        logger.info("Generated Vial JSON from %s", keyboard_json)
    except Exception:
        logger.exception("Failed to generate Vial JSON from %s", keyboard_json)
        raise typer.Exit(code=1) from None


def generate_vial(
    keyboard_json: Path,
    layout_name: str,
    *,
    keymap_c: Path | None = None,
    keyboard_config: Path | None = None,
    keyboard_id: int | None = None,
    pixels_per_unit: int = 64,
) -> VialJson:
    """Convert QMK keyboard.json to a Vial-compatible JSON structure."""
    keyboard_data = parse_json(KeyboardJson, keyboard_json)
    if (keyboard_config is None) != (keyboard_id is None):
        raise ValueError("keyboard_config and keyboard_id must be provided together")

    vendor_id = keyboard_data.usb.vid
    product_id = keyboard_data.usb.pid

    matrix_rows, matrix_cols = keyboard_data.matrix_dimensions()

    layout_data = keyboard_data.layout_keys(layout_name)
    rows_by_y = _group_layout_rows(layout_data)
    kle_rows = _build_kle_rows(rows_by_y)
    _append_encoder_row(kle_rows, keyboard_data.encoder_count())

    return VialJson(
        name=keyboard_data.keyboard_name,
        vendorId=vendor_id,
        productId=product_id,
        matrix=VialMatrix(rows=matrix_rows, cols=matrix_cols),
        layouts=VialLayouts(keymap=kle_rows),
        customKeycodes=(
            _build_custom_keycodes(keymap_c) if keymap_c is not None else None
        ),
        keymapOverlay=(
            KeymapOverlayMetadata(
                keyboardId=keyboard_id,
                layoutName=layout_name,
                pixelsPerUnit=pixels_per_unit,
                keyboard=keyboard_data,
                config=parse_json(KeyboardConfig, keyboard_config),
            )
            if keyboard_config is not None and keyboard_id is not None
            else None
        ),
    )


def _build_custom_keycodes(keymap_c: Path) -> list[VialCustomKeycode]:
    """Embed each custom keycode's identity so the device is self-describing."""
    short_names = parse_custom_keycode_short_names(keymap_c)
    return [
        VialCustomKeycode(name=name, shortName=short_names.get(name, ""))
        for name in parse_custom_keycode_names(keymap_c)
    ]


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


def _append_encoder_row(kle_rows: KleLayout, encoder_count: int) -> None:
    """Append Vial's virtual counter-clockwise and clockwise encoder keys."""
    if encoder_count == 0:
        return

    encoder_row: KleRow = []
    for index in range(encoder_count):
        if index > 0:
            encoder_row.append(KleKeyProps(x=ENCODER_PAIR_GAP))
        encoder_row.extend(
            [
                _encoder_legend(index, 0),
                _encoder_legend(index, 1),
            ]
        )
    kle_rows.append(encoder_row)


def _encoder_legend(index: int, direction: int) -> str:
    """Return the KLE legend Vial uses to identify an encoder direction."""
    return f"{index},{direction}{VIAL_ENCODER_LEGEND_SUFFIX}"


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

#!/usr/bin/env python3
# Copyright 2025 sunaemon
# SPDX-License-Identifier: MIT
from pathlib import Path
from typing import Annotated

import typer

from src.types import QmkKeymapJson
from src.util import get_logger

logger = get_logger(__name__)

app = typer.Typer()


def count_layers(qmk_keymap_json: Path) -> int:
    try:
        keymap = QmkKeymapJson.model_validate_json(qmk_keymap_json.read_text())
    except Exception as e:
        raise RuntimeError(f"Failed to parse QMK keymap JSON: {e}") from e

    layers = keymap.layers
    if not layers:
        return 0

    return len(layers)


@app.command()
def main(
    qmk_keymap_json: Annotated[
        Path, typer.Argument(help="Path to the QMK keymap JSON file")
    ],
) -> None:
    try:
        print(count_layers(qmk_keymap_json))
    except Exception:
        logger.exception("Failed to count layers in %s", qmk_keymap_json)
        raise typer.Exit(code=1) from None


if __name__ == "__main__":
    app()

# Copyright 2025 sunaemon
# SPDX-License-Identifier: MIT
import logging
from pathlib import Path
from typing import Annotated

import typer

from src.types import QmkKeymapJson, parse_json
from src.util import initialize_logging

logger = logging.getLogger(__name__)

app = typer.Typer()


@app.command()
def main(
    qmk_keymap_json: Annotated[
        Path, typer.Argument(help="Path to the QMK keymap JSON file")
    ],
) -> None:
    """Count layers in a QMK keymap JSON file and print the result."""
    initialize_logging()
    try:
        print(count_layers(qmk_keymap_json))
    except Exception:
        logger.exception("Failed to count layers in %s", qmk_keymap_json)
        raise typer.Exit(code=1) from None


def count_layers(qmk_keymap_json: Path) -> int:
    """Return the number of layers in a QMK keymap JSON file."""
    qmk_keymap_data = parse_json(QmkKeymapJson, qmk_keymap_json)

    layers = qmk_keymap_data.layers
    if not layers:
        return 0

    return len(layers)


if __name__ == "__main__":
    app()

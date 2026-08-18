# Copyright 2026 sunaemon
# SPDX-License-Identifier: MIT
import logging
from collections.abc import Iterable
from pathlib import Path
from typing import Annotated

import typer

from model.src.types import KeyboardJson, parse_json
from model.src.util import initialize_logging

logger = logging.getLogger(__name__)

app = typer.Typer()

FULL_RECURSION = "--recursive"
# Every processor here builds on ChibiOS, not AVR, so none of them has its own
# platforms/<platform>/printf.mk; builddefs/common_features.mk falls back to
# lib/printf whenever that file is missing.
PROCESSOR_SUBMODULES = {
    "RP2040": frozenset(
        {"lib/chibios", "lib/chibios-contrib", "lib/lufa", "lib/pico-sdk", "lib/printf"}
    ),
    "STM32F103": frozenset(
        {"lib/chibios", "lib/chibios-contrib", "lib/lufa", "lib/printf"}
    ),
}


@app.command()
def main(
    keyboards_dir: Annotated[
        Path, typer.Argument(help="Directory containing keyboard configurations")
    ],
) -> None:
    """Print the Vial QMK submodules required by the configured keyboards."""
    initialize_logging()
    try:
        submodules, unknown_processors = resolve_qmk_submodules(
            sorted(keyboards_dir.glob("*/keyboard.json"))
        )
    except Exception:
        logger.exception("Failed to resolve QMK dependencies from %s", keyboards_dir)
        raise typer.Exit(code=1) from None

    if unknown_processors:
        processors = ", ".join(unknown_processors)
        logger.warning(
            "Unknown QMK processor(s) %s; initializing every nested submodule",
            processors,
        )
        print(FULL_RECURSION)
        return
    print(" ".join(submodules))


def resolve_qmk_submodules(
    keyboard_json_paths: Iterable[Path],
) -> tuple[list[str], list[str]]:
    """Return required submodules and processors that need the safe fallback."""
    submodules: set[str] = set()
    unknown_processors: set[str] = set()
    found_keyboard = False

    for path in keyboard_json_paths:
        found_keyboard = True
        processor = parse_json(KeyboardJson, path).processor
        required = PROCESSOR_SUBMODULES.get(processor or "")
        if required is None:
            unknown_processors.add(processor or "<missing>")
        else:
            submodules.update(required)

    if not found_keyboard:
        unknown_processors.add("<no keyboards>")
    return sorted(submodules), sorted(unknown_processors)


if __name__ == "__main__":
    app()

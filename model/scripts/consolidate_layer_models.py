# Copyright 2026 sunaemon
# SPDX-License-Identifier: MIT
import json
import logging
import re
from pathlib import Path
from typing import Annotated

import typer
from pydantic import BaseModel, ConfigDict, StrictInt

from model.src.util import initialize_logging, write_stdout_bytes

logger = logging.getLogger(__name__)

app = typer.Typer()


class LayerModelEnvelope(BaseModel):
    """Validate a rendered model's layer identity while preserving its fields."""

    model_config = ConfigDict(extra="allow")
    layer: StrictInt


@app.command()
def main(
    keyboard_id: Annotated[
        int, typer.Option(min=0, max=255, help="Keyboard ID these layers belong to")
    ],
    layer_json: Annotated[
        list[Path],
        typer.Option("--layer-json", help="Path to one rendered layer's JSON model"),
    ],
) -> None:
    """Combine one keyboard's rendered layer models into a single installable file."""
    initialize_logging()
    try:
        combined = consolidate_layer_models(keyboard_id, layer_json)
        write_stdout_bytes(
            (
                json.dumps(combined, ensure_ascii=False, separators=(",", ":")) + "\n"
            ).encode()
        )
        logger.info(
            "Consolidated %d layers for keyboard %d",
            len(combined["layers"]),
            keyboard_id,
        )
    except Exception:
        logger.exception("Failed to consolidate layers for keyboard %d", keyboard_id)
        raise typer.Exit(code=1) from None


def consolidate_layer_models(keyboard_id: int, layer_json_paths: list[Path]) -> dict:
    """Combine a keyboard's rendered layer files into one object."""
    # Take the exact paths Make considers current rather than globbing, so a
    # stale leftover from a shrunk layer count never sneaks in.
    layer_pattern = re.compile(rf"^{keyboard_id}_L(\d+)$")
    layers: dict[str, object] = {}
    for path in layer_json_paths:
        match = layer_pattern.fullmatch(path.stem)
        if match is None:
            raise ValueError(
                f"{path} is not a rendered layer for keyboard {keyboard_id}"
            )
        filename_layer = int(match.group(1))
        model = LayerModelEnvelope.model_validate_json(path.read_text(encoding="utf-8"))
        model_layer = model.layer
        if model_layer != filename_layer:
            raise ValueError(f"Layer in {path} does not match its filename")
        key = str(model_layer)
        if key in layers:
            raise ValueError(
                f"Layer {model_layer} is defined more than once (duplicate {path})"
            )
        layers[key] = model.model_dump(mode="json")
    if not layers:
        raise ValueError(f"No layer models given for keyboard {keyboard_id}")
    return {"keyboard_id": keyboard_id, "layers": layers}


if __name__ == "__main__":
    app()

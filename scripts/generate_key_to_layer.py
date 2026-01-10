# Copyright 2025 sunaemon
# SPDX-License-Identifier: MIT
import logging
import re
from pathlib import Path
from typing import Annotated

import typer

from src.types import KeyToLayerJson, print_json
from src.util import initialize_logging, strip_c_comments

logger = logging.getLogger(__name__)

app = typer.Typer()


@app.command()
def main(
    keymap_c: Annotated[Path, typer.Option(help="Path to keymap.c")],
    prefix: Annotated[str, typer.Option(help="Prefix for layer names")] = "",
) -> None:
    """Generate key-to-layer JSON from keymap.c and emit it to stdout."""
    initialize_logging()
    try:
        key_to_layer = _process(keymap_c, prefix)
        print_json(key_to_layer)
        logger.info("Generated key-to-layer mapping.")
    except Exception:
        logger.exception("Failed to generate key-to-layer mapping from %s", keymap_c)
        raise typer.Exit(code=1) from None


def _process(keymap_c: Path, prefix: str) -> KeyToLayerJson:
    content = keymap_c.read_text()
    notifier_keys = _parse_notifier_keys(content)
    return _build_mapping(notifier_keys, prefix)


def _parse_notifier_keys(content: str) -> list[str]:
    match = re.search(
        r"notifier_key_to_layer\s*\[[^\]]+\]\s*=\s*\{(.*?)\};",
        content,
        flags=re.DOTALL,
    )
    if not match:
        raise ValueError("notifier_key_to_layer array not found")

    raw_entries = strip_c_comments(match.group(1))
    notifier_keys = [entry.strip() for entry in raw_entries.split(",") if entry.strip()]
    if not notifier_keys:
        raise ValueError("notifier_key_to_layer array is empty")

    return notifier_keys


def _build_mapping(notifier_keys: list[str], prefix: str) -> KeyToLayerJson:
    mapping: dict[str, str] = {}
    for idx, entry in enumerate(notifier_keys):
        key_name = _normalize_keycode(entry)
        mapping[key_name] = f"{prefix}L{idx}"
    return KeyToLayerJson.model_validate(mapping)


def _normalize_keycode(entry: str) -> str:
    match = re.fullmatch(r"(?:KC_)?F(\d+)", entry)
    if not match:
        raise ValueError(f"Unsupported notifier keycode: {entry}")
    return f"f{int(match.group(1))}"


if __name__ == "__main__":
    app()

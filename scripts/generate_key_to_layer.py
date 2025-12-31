#!/usr/bin/env python3
# Copyright 2025 sunaemon
# SPDX-License-Identifier: MIT
import re
from pathlib import Path
from typing import Annotated

import typer

from src.types import KeyToLayerJson
from src.util import get_logger

logger = get_logger(__name__)

app = typer.Typer()


def strip_comments(text: str) -> str:
    text = re.sub(r"/\*.*?\*/", "", text, flags=re.DOTALL)
    text = re.sub(r"//[^\n]*", "", text)
    return text


def parse_notifier_keys(content: str) -> list[str]:
    match = re.search(
        r"notifier_key_to_layer\s*\[[^\]]+\]\s*=\s*\{(.*?)\};",
        content,
        flags=re.DOTALL,
    )
    if not match:
        raise ValueError("notifier_key_to_layer array not found")

    raw_entries = strip_comments(match.group(1))
    notifier_keys = [entry.strip() for entry in raw_entries.split(",") if entry.strip()]
    if not notifier_keys:
        raise ValueError("notifier_key_to_layer array is empty")

    return notifier_keys


def normalize_keycode(entry: str) -> str:
    match = re.fullmatch(r"(?:KC_)?F(\d+)", entry)
    if not match:
        raise ValueError(f"Unsupported notifier keycode: {entry}")
    return f"f{int(match.group(1))}"


def build_mapping(notifier_keys: list[str]) -> KeyToLayerJson:
    mapping: dict[str, str] = {}
    for idx, entry in enumerate(notifier_keys):
        key_name = normalize_keycode(entry)
        mapping[key_name] = f"L{idx}"
    return KeyToLayerJson.model_validate(mapping)


def process(keymap_c: Path, key_to_layer_json: Path) -> None:
    content = keymap_c.read_text()
    try:
        notifier_keys = parse_notifier_keys(content)
        key_to_layer_content = build_mapping(notifier_keys)
    except ValueError as exc:
        raise ValueError(f"Failed to parse {keymap_c}") from exc

    key_to_layer_json.write_text(key_to_layer_content.model_dump_json(indent=4) + "\n")


@app.command()
def main(
    keymap_c: Annotated[Path, typer.Option(help="Path to keymap.c")],
    key_to_layer_json: Annotated[
        Path, typer.Option(help="Path to output key-to-layer.json")
    ],
) -> None:
    """
    Generate key-to-layer.json from keymap.c
    """

    try:
        process(keymap_c, key_to_layer_json)
        logger.info(f"Successfully generated {key_to_layer_json}")
    except Exception:
        logger.exception("Failed to generate key-to-layer mapping from %s", keymap_c)
        raise typer.Exit(code=1) from None


if __name__ == "__main__":
    app()

# Copyright 2026 sunaemon
# SPDX-License-Identifier: MIT
import re
from pathlib import Path

from src.util import strip_c_comments

ENCODER_PAIR_NAME = "ENCODER_CCW_CW"


def parse_encoder_map(keymap_c: Path) -> list[list[list[str]]]:
    """Parse QMK encoder bindings from keymap.c."""
    content = strip_c_comments(keymap_c.read_text(encoding="utf-8"))
    name = re.search(r"\bencoder_map\b", content)
    if name is None:
        return []
    equals = content.find("=", name.end())
    opening = content.find("{", equals + 1)
    if equals < 0 or opening < 0:
        raise ValueError(f"Malformed encoder_map in {keymap_c}")
    body, _ = _extract_delimited(content, opening, "{", "}")
    return _parse_encoder_layers(body, keymap_c)


def _parse_encoder_layers(content: str, keymap_c: Path) -> list[list[list[str]]]:
    indexed_layers: dict[int, list[list[str]]] = {}
    position = 0
    layer_pattern = re.compile(r"\[(\d+)\]\s*=")
    while match := layer_pattern.search(content, position):
        opening = content.find("{", match.end())
        if opening < 0:
            raise ValueError(f"Malformed encoder_map layer in {keymap_c}")
        body, position = _extract_delimited(content, opening, "{", "}")
        layer_index = int(match.group(1))
        if layer_index in indexed_layers:
            raise ValueError(f"Duplicate encoder_map layer {layer_index} in {keymap_c}")
        indexed_layers[layer_index] = _parse_encoder_pairs(body, keymap_c)
    if not indexed_layers:
        return []
    layers = [[] for _ in range(max(indexed_layers) + 1)]
    for layer_index, pairs in indexed_layers.items():
        layers[layer_index] = pairs
    return layers


def _parse_encoder_pairs(content: str, keymap_c: Path) -> list[list[str]]:
    pairs: list[list[str]] = []
    position = 0
    while (start := content.find(ENCODER_PAIR_NAME, position)) >= 0:
        opening = content.find("(", start + len(ENCODER_PAIR_NAME))
        if opening < 0:
            raise ValueError(f"Malformed {ENCODER_PAIR_NAME} in {keymap_c}")
        arguments, position = _extract_delimited(content, opening, "(", ")")
        pairs.append(_split_pair(arguments, keymap_c))
    return pairs


def _split_pair(arguments: str, keymap_c: Path) -> list[str]:
    depth = 0
    separators = []
    for index, character in enumerate(arguments):
        if character == "(":
            depth += 1
        elif character == ")":
            depth -= 1
        elif character == "," and depth == 0:
            separators.append(index)
    if len(separators) != 1 or depth != 0:
        raise ValueError(f"Malformed {ENCODER_PAIR_NAME} arguments in {keymap_c}")
    separator = separators[0]
    return [arguments[:separator].strip(), arguments[separator + 1 :].strip()]


def _extract_delimited(
    content: str,
    opening_index: int,
    opening_character: str,
    closing_character: str,
) -> tuple[str, int]:
    depth = 0
    for index in range(opening_index, len(content)):
        character = content[index]
        if character == opening_character:
            depth += 1
        elif character == closing_character:
            depth -= 1
            if depth == 0:
                return content[opening_index + 1 : index], index + 1
    raise ValueError(f"Unclosed {opening_character} in encoder_map")

# Copyright 2025 sunaemon
# SPDX-License-Identifier: MIT
import logging
import os
import re
import sys
from pathlib import Path

from src.types import KeyboardJson, LayoutKey, parse_json

# Configure logging to stderr so it doesn't interfere with stdout redirection
log_level = os.environ.get("LOG_LEVEL", "INFO").upper()
logging.basicConfig(
    level=getattr(logging, log_level, logging.INFO),
    format="%(levelname)s: %(message)s",
    stream=sys.stderr,
)


def get_logger(name: str) -> logging.Logger:
    """
    Returns a logger instance with the given name.
    """
    return logging.getLogger(name)


def strip_c_comments(text: str) -> str:
    """
    Remove C-style // and /* */ comments from text.
    """
    text = re.sub(r"/\*.*?\*/", "", text, flags=re.DOTALL)
    text = re.sub(r"//[^\n]*", "", text)
    return text


def get_layout_keys(
    keyboard_data: KeyboardJson,
    layout_name: str,
) -> list[LayoutKey]:
    layouts = keyboard_data.layouts
    if layout_name not in layouts:
        raise ValueError(f"Layout {layout_name} not found in keyboard.json")
    return layouts[layout_name].layout


def load_layout_keys(
    keyboard_json: Path,
    layout_name: str,
) -> list[LayoutKey]:
    keyboard_data = parse_json(KeyboardJson, keyboard_json)
    return get_layout_keys(keyboard_data, layout_name)

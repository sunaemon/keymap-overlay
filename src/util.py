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
    """Returns a logger instance with the given name."""
    return logging.getLogger(name)


def strip_c_comments(text: str) -> str:
    """Remove C-style // and /* */ comments from text."""
    text = re.sub(r"/\*.*?\*/", "", text, flags=re.DOTALL)
    text = re.sub(r"//[^\n]*", "", text)
    return text


def parse_hex_keycode(key: str) -> int | None:
    """Parse a hex keycode string like 0x1A2B into an int."""
    if key.startswith(("0x", "0X")):
        try:
            return int(key, 16)
        except ValueError:
            return None
    return None


def parse_keycode_value(key: str) -> int | None:
    """Parse hex or decimal keycode string into an int."""
    value = parse_hex_keycode(key)
    if value is not None:
        return value
    if key.isdigit():
        try:
            return int(key)
        except ValueError:
            return None
    return None


def load_layout_keys(
    keyboard_json: Path,
    layout_name: str,
) -> list[LayoutKey]:
    """Load keyboard.json and return layout keys for a named layout."""
    keyboard_data = parse_json(KeyboardJson, keyboard_json)
    return keyboard_data.layout_keys(layout_name)

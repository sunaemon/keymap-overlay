# Copyright 2025 sunaemon
# SPDX-License-Identifier: MIT
import logging
import os
import re
import sys
from pathlib import Path

from model.src.types import KeyboardJson, LayoutKey, VialCustomKeycode, parse_json

# An enum custom_keycodes entry must start at the keyboard-specific range that
# Vial uses when mapping embedded labels back to dynamic keycodes.
CUSTOM_KEYCODE_BASE_NAME = "QK_KB_0"

# Vial requires custom keycodes to be assigned starting at QK_KB_0; a device's
# embedded definition carries no numeric base of its own to look up.
VIAL_CUSTOM_KEYCODE_BASE = 0x7E00
_QK_KB_KEYCODE_PATTERN = re.compile(r"QK_KB_(\d+)")


def parse_qk_kb_keycode(name: str) -> int | None:
    """Parse vitaly's generic QK_KB_<n> keycode name into its numeric value."""
    match = _QK_KB_KEYCODE_PATTERN.fullmatch(name.strip())
    return VIAL_CUSTOM_KEYCODE_BASE + int(match.group(1)) if match else None


def write_stdout_bytes(data: bytes) -> None:
    """Writes raw bytes to stdout, bypassing the locale codepage on Windows."""
    buffer = getattr(sys.stdout, "buffer", None)
    if buffer is None:
        raise OSError("Binary stdout is unavailable")
    sys.stdout.flush()
    buffer.write(data)
    buffer.flush()


def initialize_logging() -> None:
    """Initialize logging to stderr for CLI scripts."""
    log_level = os.environ.get("LOG_LEVEL", "INFO").upper()
    logging.basicConfig(
        level=getattr(logging, log_level, logging.INFO),
        format="%(levelname)s: %(message)s",
        stream=sys.stderr,
    )


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


def parse_custom_keycodes(keymap_c: Path) -> list[VialCustomKeycode]:
    """Parse validated custom keycode identities and labels from keymap.c."""
    content = keymap_c.read_text(encoding="utf-8")
    match = re.search(
        r"enum\s+custom_keycodes\s*\{([^}]*)\};", content, re.DOTALL | re.MULTILINE
    )
    if match is None:
        return []

    body = match.group(1)
    short_names: dict[str, str] = {}
    # Requiring one whitespace-free token distinguishes a label such as "α" or
    # "USB-C" from a prose comment explaining the entry.
    for line in body.splitlines():
        label = re.fullmatch(
            r"\s*([A-Za-z_]\w*)(?:\s*=\s*[^,]+)?\s*,?\s*//\s*(\S+)\s*",
            line,
        )
        if label:
            short_names[label.group(1)] = label.group(2)

    entries = [
        entry.strip() for entry in strip_c_comments(body).split(",") if entry.strip()
    ]

    custom_keycodes: list[VialCustomKeycode] = []
    for index, entry in enumerate(entries):
        if "=" in entry:
            name, value = (part.strip() for part in entry.split("=", 1))
            if value != CUSTOM_KEYCODE_BASE_NAME:
                raise ValueError(
                    f"Custom keycodes must start at {CUSTOM_KEYCODE_BASE_NAME}: {entry}"
                )
            if index != 0:
                raise ValueError(
                    f"Custom keycode base may only be assigned to the first entry: {entry}"
                )
        else:
            if index == 0:
                raise ValueError(
                    f"Custom keycodes must start at {CUSTOM_KEYCODE_BASE_NAME}: {entry}"
                )
            name = entry
        custom_keycodes.append(
            VialCustomKeycode(name=name, shortName=short_names.get(name, ""))
        )
    return custom_keycodes

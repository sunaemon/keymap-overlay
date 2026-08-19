# Copyright 2025 sunaemon
# SPDX-License-Identifier: MIT
import logging
import os
import re
import sys
from pathlib import Path

from model.src.types import KeyboardJson, LayoutKey, parse_json

# Names an enum custom_keycodes entry may be explicitly assigned to reset the
# numbering back to the keyboard's custom-keycode base, rather than continuing
# the previous entry's value.
CUSTOM_KEYCODE_BASE_NAMES = {"SAFE_RANGE", "QK_USER_0", "QK_KB_0"}

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


def parse_custom_keycode_names(keymap_c: Path) -> list[str]:
    """Return enum custom_keycodes member names from keymap.c, in declared order."""
    content = keymap_c.read_text(encoding="utf-8")
    match = re.search(
        r"enum\s+custom_keycodes\s*\{([^}]*)\};", content, re.DOTALL | re.MULTILINE
    )
    if match is None:
        raise ValueError(f"enum custom_keycodes not found in {keymap_c}")

    entries = [
        entry.strip()
        for entry in strip_c_comments(match.group(1)).split(",")
        if entry.strip()
    ]

    names: list[str] = []
    for entry in entries:
        if "=" in entry:
            name, value = (part.strip() for part in entry.split("=", 1))
            if value not in CUSTOM_KEYCODE_BASE_NAMES:
                raise ValueError(
                    f"Explicit keycode assignment is not supported: {entry}"
                )
        else:
            name = entry
        names.append(name)
    return names


def parse_custom_keycode_short_names(keymap_c: Path) -> dict[str, str]:
    """Read single-character comment labels off enum custom_keycodes entries."""
    content = keymap_c.read_text(encoding="utf-8")
    labels: dict[str, str] = {}

    custom_keycodes = re.search(
        r"enum\s+custom_keycodes\s*\{(.*?)\};",
        content,
        re.DOTALL,
    )
    if custom_keycodes:
        for line in custom_keycodes.group(1).splitlines():
            match = re.fullmatch(
                r"\s*([A-Za-z_]\w*)(?:\s*=\s*[^,]+)?\s*,?\s*//\s*(.*?)\s*",
                line,
            )
            if match and len(match.group(2)) == 1:
                labels[match.group(1)] = match.group(2)

    return labels

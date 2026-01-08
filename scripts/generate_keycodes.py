#!/usr/bin/env python3
# Copyright 2025 sunaemon
# SPDX-License-Identifier: MIT
import os
import sys
import types
from pathlib import Path
from typing import Annotated

import typer

from src.types import KeycodesJson, QmkKeycodesSpec, print_json
from src.util import get_logger, parse_hex_keycode

logger = get_logger(__name__)

app = typer.Typer()

PREFERRED_NAMES = {
    "KC_TRNS",
    "KC_ESC",
    "KC_ENT",
    "KC_BSPC",
    "KC_LCTL",
    "KC_RCTL",
    "KC_LSFT",
    "KC_RSFT",
    "KC_LALT",
    "KC_RALT",
    "KC_LGUI",
    "KC_RGUI",
    "KC_SCLN",
    "KC_QUOT",
    "KC_COMM",
    "KC_MINS",
    "KC_EQL",
    "KC_BSLS",
    "KC_GRV",
    "KC_SLSH",
    "KC_LBRC",
    "KC_RBRC",
    "KC_SPC",
    "KC_APP",
    "KC_PSCR",
    "KC_SCRL",
    "KC_NUHS",
    "KC_NUBS",
    "KC_LCBR",
    "KC_RCBR",
    "KC_LPRN",
    "KC_RPRN",
    "KC_TILD",
    "KC_EXLM",
    "KC_AT",
    "KC_HASH",
    "KC_DLR",
    "KC_PERC",
    "KC_CIRC",
    "KC_AMPR",
    "KC_ASTR",
    "KC_UNDS",
    "KC_PLUS",
    "KC_PIPE",
    "KC_COLN",
    "KC_DQUO",
    "KC_LABK",
    "KC_RABK",
    "KC_QUES",
}


@app.command()
def main(
    qmk_dir: Annotated[
        Path,
        typer.Option(help="Path to qmk_firmware directory"),
    ],
) -> None:
    try:
        keycodes = generate_keycodes(qmk_dir)
        print_json(keycodes)
        logger.info(
            "Generated %d keycodes JSON from QMK firmware at %s",
            len(keycodes.root),
            qmk_dir,
        )
    except Exception:
        logger.exception("Failed to generate keycodes JSON")
        raise typer.Exit(code=1) from None


def generate_keycodes(qmk_dir: Path) -> KeycodesJson:
    """Generate the keycodes JSON from QMK firmware sources."""
    spec = _read_latest_qmk_spec(qmk_dir)

    code_to_name: dict[int, str] = {}

    for hex_code, info in spec.keycodes.items():
        code = parse_hex_keycode(hex_code)
        if code is None:
            logger.warning("Invalid hex keycode in spec: %s", hex_code)
            continue

        names = [info.key]
        if info.aliases:
            names.extend(info.aliases)

        names = sorted(names, key=_name_rank)
        code_to_name[code] = names[0]

    sorted_codes = sorted(code_to_name.keys())

    output_dict: dict[str, str] = {}
    for code in sorted_codes:
        output_dict[f"0x{code:04X}"] = code_to_name[code]

    return KeycodesJson.model_validate(output_dict)


def _read_latest_qmk_spec(qmk_dir: Path) -> QmkKeycodesSpec:
    qmk_lib_path = qmk_dir / "lib" / "python"
    if not qmk_lib_path.exists():
        raise FileNotFoundError(f"QMK lib path not found at {qmk_lib_path}")

    sys.path.insert(0, str(qmk_lib_path))
    sys.modules["milc"] = types.ModuleType("milc")  # Dummy module for milc dependency
    sys.modules["milc.cli"] = types.ModuleType(
        "milc.cli"
    )  # Dummy module for milc dependency
    import qmk.keycodes as qmk_keycodes  # type: ignore

    original_cwd = Path.cwd()
    os.chdir(qmk_dir)

    try:
        versions = qmk_keycodes.list_versions()
        if versions is None or not versions:
            raise ValueError("No QMK keycodes versions found")
        # Load the latest version
        raw_spec = qmk_keycodes.load_spec(versions[-1])

        # Use Pydantic to validate the spec
        spec = QmkKeycodesSpec.model_validate(raw_spec)
        return spec
    finally:
        os.chdir(original_cwd)


def _name_rank(name: str) -> tuple[int, int]:
    if name in PREFERRED_NAMES:
        return (0, len(name))
    if name == "KC_NO":
        return (1, len(name))
    return (2, len(name))


if __name__ == "__main__":
    app()

#!/usr/bin/env python3
# Copyright 2025 sunaemon
# SPDX-License-Identifier: MIT
import os
import sys
import types
from pathlib import Path
from typing import Annotated

import typer

from src.types import KeycodesJson, QmkKeycodeSpecEntry, QmkKeycodesSpec, print_json
from src.util import get_logger

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


def _choose_best_name(current: str | None, candidate: str) -> str:
    if current is None:
        return candidate
    return min(current, candidate, key=_name_rank)


def _parse_hex_code(hex_code: str) -> int | None:
    try:
        return int(hex_code, 16)
    except ValueError:
        return None


def _get_names(info: QmkKeycodeSpecEntry) -> list[str]:
    names: list[str | None] = [info.key]
    if info.aliases:
        names.extend(info.aliases)
    return [name for name in names if name]


def generate_keycodes(qmk_dir: Path) -> KeycodesJson:
    """Generate the keycodes JSON from QMK firmware sources."""
    spec = _read_latest_qmk_spec(qmk_dir)

    code_to_name: dict[int, str] = {}

    # New structure: "0x0004": { "key": "KC_A", "aliases": [...] }
    for hex_code, info in spec.keycodes.items():
        code = _parse_hex_code(hex_code)
        if code is None:
            continue

        names = _get_names(info)
        if not names:
            continue

        best = names[0]
        for name in names[1:]:
            best = _choose_best_name(best, name)

        code_to_name[code] = _choose_best_name(code_to_name.get(code), best)

    sorted_codes = sorted(code_to_name.keys())

    output_dict: dict[str, str] = {}
    for code in sorted_codes:
        output_dict[f"0x{code:04X}"] = code_to_name[code]

    keycodes_model = KeycodesJson.model_validate(output_dict)
    logger.info("Generated %d keycodes.", len(sorted_codes))
    return keycodes_model


@app.command()
def main(
    qmk_dir: Annotated[
        Path,
        typer.Option(help="Path to qmk_firmware directory"),
    ],
) -> None:
    try:
        print_json(generate_keycodes(qmk_dir))
        logger.info("Generated keycodes JSON from QMK firmware at %s", qmk_dir)
    except Exception:
        logger.exception("Failed to generate keycodes JSON")
        raise typer.Exit(code=1) from None


if __name__ == "__main__":
    app()

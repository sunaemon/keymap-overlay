#!/usr/bin/env python3
# Copyright 2025 sunaemon
# SPDX-License-Identifier: MIT
import os
import sys
from pathlib import Path
from typing import Annotated

import typer

from src.types import KeycodesJson, QmkKeycodesSpec
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


def read_latest_qmk_spec(qmk_dir: Path) -> QmkKeycodesSpec:
    qmk_lib_path = qmk_dir / "lib" / "python"
    if not qmk_lib_path.exists():
        raise FileNotFoundError(f"QMK lib path not found at {qmk_lib_path}")

    sys.path.insert(0, str(qmk_lib_path))
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


def generate_keycodes_file(qmk_dir: Path, keycodes_json: Path) -> None:
    spec = read_latest_qmk_spec(qmk_dir)

    code_to_name: dict[int, str] = {}

    # New structure: "0x0004": { "key": "KC_A", "aliases": [...] }
    for hex_code, info in spec.keycodes.items():
        try:
            code = int(hex_code, 16)
        except ValueError:
            continue

        names: list[str | None] = [info.key]
        if info.aliases:
            names.extend(info.aliases)

        for name in names:
            if not name:
                continue

            if code in code_to_name:
                current_name = code_to_name[code]
                if current_name in PREFERRED_NAMES:
                    continue
                if name in PREFERRED_NAMES:
                    code_to_name[code] = name
                    continue
                if name == "KC_TRNS" or name == "KC_NO":
                    code_to_name[code] = name
                elif current_name == "KC_TRNS" or current_name == "KC_NO":
                    pass
                else:
                    if len(name) < len(current_name):
                        code_to_name[code] = name
            else:
                code_to_name[code] = name

    sorted_codes = sorted(code_to_name.keys())

    output_dict: dict[str, str] = {}
    for code in sorted_codes:
        output_dict[f"0x{code:04X}"] = code_to_name[code]

    keycodes_model = KeycodesJson.model_validate(output_dict)
    try:
        keycodes_json.write_text(keycodes_model.model_dump_json(indent=4) + "\n")
        logger.info(
            f"Successfully generated {keycodes_json} with {len(sorted_codes)} keycodes."
        )
    except OSError as exc:
        raise OSError(f"Failed to write {keycodes_json}") from exc


@app.command()
def main(
    qmk_dir: Annotated[
        Path,
        typer.Option(help="Path to qmk_firmware directory"),
    ],
    keycodes_json: Annotated[
        Path,
        typer.Option(help="Path to output JSON file"),
    ],
) -> None:
    """
    Generate keycodes.json from QMK Firmware
    """
    try:
        generate_keycodes_file(qmk_dir, keycodes_json)
    except Exception:
        logger.exception("Failed to generate keycodes JSON: %s", keycodes_json)
        raise typer.Exit(code=1) from None


if __name__ == "__main__":
    app()

# Copyright 2025 sunaemon
# SPDX-License-Identifier: MIT
import json
from pathlib import Path

from scripts.postprocess_qmk_keymap import postprocess_qmk_keymap


def test_postprocess_qmk_keymap_resolves_custom_and_transparency(
    tmp_path: Path,
) -> None:
    qmk_keymap = {
        "version": 1,
        "layout": "LAYOUT",
        "layers": [["0x0004", "KC_B"], ["KC_TRNS", "KC_TRNS"]],
    }
    custom_keycodes = {"0x0004": "KC_ALPHA"}

    qmk_keymap_path = tmp_path / "qmk-keymap.json"
    custom_keycodes_path = tmp_path / "custom-keycodes.json"

    qmk_keymap_path.write_text(json.dumps(qmk_keymap))
    custom_keycodes_path.write_text(json.dumps(custom_keycodes))

    keymap = postprocess_qmk_keymap(qmk_keymap_path, custom_keycodes_path)

    assert keymap.layers == [["KC_ALPHA", "KC_B"], ["KC_ALPHA", "KC_B"]]

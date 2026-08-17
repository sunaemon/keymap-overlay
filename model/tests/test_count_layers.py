# Copyright 2026 sunaemon
# SPDX-License-Identifier: MIT
import json
from pathlib import Path

import pytest

from model.scripts.count_layers import count_layers
from model.src.types import JSONParseError, JSONReadError

DATA_DIR = Path(__file__).parent / "data"


def test_count_layers_reads_the_keymap() -> None:
    """The Makefile expands this into one drawing target per layer."""
    assert count_layers(DATA_DIR / "qmk-keymap.json") == 2


def test_a_keymap_with_no_layers_counts_zero(tmp_path: Path) -> None:
    qmk_keymap = tmp_path / "qmk-keymap.json"
    qmk_keymap.write_text(json.dumps({"version": 1, "layers": []}))

    assert count_layers(qmk_keymap) == 0


def test_a_missing_keymap_is_reported_with_its_path(tmp_path: Path) -> None:
    with pytest.raises(JSONReadError, match="qmk-keymap.json"):
        count_layers(tmp_path / "qmk-keymap.json")


def test_a_malformed_keymap_is_reported_with_its_path(tmp_path: Path) -> None:
    qmk_keymap = tmp_path / "qmk-keymap.json"
    qmk_keymap.write_text("{not json")

    with pytest.raises(JSONParseError, match="qmk-keymap.json"):
        count_layers(qmk_keymap)

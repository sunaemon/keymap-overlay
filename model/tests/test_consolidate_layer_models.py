# Copyright 2026 sunaemon
# SPDX-License-Identifier: MIT
import json
from pathlib import Path

import pytest

from model.scripts.consolidate_layer_models import consolidate_layer_models


def _write_layer(
    directory: Path, keyboard_id: int, layer: int, extra: dict | None = None
) -> Path:
    model = {
        "version": 2,
        "layer": layer,
        "width": 10,
        "height": 10,
        "keys": [],
        "encoders": [],
    }
    model.update(extra or {})
    path = directory / f"{keyboard_id}_L{layer}.json"
    path.write_text(json.dumps(model), encoding="utf-8")
    return path


def test_combines_every_given_layer(tmp_path: Path) -> None:
    paths = [
        _write_layer(tmp_path, 1, 0),
        _write_layer(tmp_path, 1, 1),
        _write_layer(tmp_path, 1, 2),
    ]

    combined = consolidate_layer_models(1, paths)

    assert combined["keyboard_id"] == 1
    assert set(combined["layers"]) == {"0", "1", "2"}
    assert combined["layers"]["1"]["layer"] == 1


def test_ignores_a_stale_file_that_is_not_in_the_given_list(tmp_path: Path) -> None:
    """A leftover from a shrunk layer count must not sneak into the output."""
    current = [_write_layer(tmp_path, 1, 0), _write_layer(tmp_path, 1, 1)]
    _write_layer(tmp_path, 1, 2)  # stale on disk, but not passed in

    combined = consolidate_layer_models(1, current)

    assert set(combined["layers"]) == {"0", "1"}


def test_rejects_a_layer_field_that_does_not_match_its_filename(tmp_path: Path) -> None:
    path = _write_layer(tmp_path, 1, 0, extra={"layer": 5})

    with pytest.raises(ValueError, match="does not match its filename"):
        consolidate_layer_models(1, [path])


def test_rejects_a_path_that_is_not_a_layer_for_this_keyboard(tmp_path: Path) -> None:
    other_keyboard_path = _write_layer(tmp_path, 2, 0)

    with pytest.raises(ValueError, match="is not a rendered layer for keyboard 1"):
        consolidate_layer_models(1, [other_keyboard_path])


def test_rejects_an_empty_list(tmp_path: Path) -> None:
    with pytest.raises(ValueError, match="No layer models given"):
        consolidate_layer_models(1, [])

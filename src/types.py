# Copyright 2025 sunaemon
# SPDX-License-Identifier: MIT
from pathlib import Path
from typing import Annotated, Type, TypeVar

from pydantic import BaseModel, ConfigDict, Field, RootModel


class BaseModelAllow(BaseModel):
    model_config = ConfigDict(extra="allow")


class QmkKeycodeSpecEntry(BaseModelAllow):
    key: str | None = None
    aliases: list[str] | None = None


class QmkKeycodesSpec(BaseModelAllow):
    keycodes: Annotated[dict[str, QmkKeycodeSpecEntry], Field(default_factory=dict)]


class LayoutKey(BaseModelAllow):
    x: float
    y: float
    matrix: Annotated[tuple[int, int], Field(description="Row, Column")]
    w: float = 1.0
    h: float = 1.0
    label: str | None = None


class Layout(BaseModelAllow):
    layout: list[LayoutKey]


class Features(BaseModelAllow):
    bootmagic: bool | None = None
    mousekey: bool | None = None
    extrakey: bool | None = None
    console: bool | None = None
    command: bool | None = None
    nkro: bool | None = None


class USBConfig(BaseModelAllow):
    vid: str
    pid: str
    device_version: str


class MatrixPins(BaseModelAllow):
    cols: list[str | None] | None = None
    rows: list[str | None] | None = None


class SplitConfig(BaseModelAllow):
    enabled: bool = False
    matrix_pins: dict[str, MatrixPins] | None = None


class KeyboardJson(BaseModelAllow):
    keyboard_name: str
    layouts: dict[str, Layout]
    manufacturer: str | None = None
    maintainer: str | None = None
    url: str | None = None
    processor: str | None = None
    bootloader: str | None = None
    diode_direction: str | None = None
    features: Features | None = None
    usb: USBConfig | None = None
    matrix_pins: MatrixPins | None = None
    split: SplitConfig | None = None


class QmkKeymapJson(BaseModelAllow):
    version: int | None = None
    # dimesion: layer -> flattened index
    layers: list[list[str]] | None = None
    layout: str | None = None


class KeycodesJson(RootModel[dict[str, str]]):
    pass


class CustomKeycodesJson(RootModel[dict[str, str]]):
    pass


class VialMatrix(BaseModelAllow):
    rows: int
    cols: int


class VialLayouts(BaseModelAllow):
    keymap: list[list[str | dict[str, float]]]


class VialJson(BaseModelAllow):
    name: str
    vendorId: str
    productId: str
    matrix: VialMatrix
    layouts: VialLayouts


class VitalyJson(BaseModelAllow):
    # dimensions: layer -> row -> col
    # cf. https://github.com/bskaplou/vitaly/blob/93f08de4b6022007f4e3e655b6d76682e275f4cc/src/protocol.rs#L454
    layout: list[list[list[str]]]


class KeyToLayerJson(RootModel[dict[str, str]]):
    pass


T = TypeVar("T", bound=BaseModel)


def parse_json(model: Type[T], path: Path) -> T:
    try:
        return model.model_validate_json(path.read_text())
    except OSError as e:
        raise RuntimeError(f"Failed to read JSON from {path}") from e
    except Exception as e:
        raise RuntimeError(f"Failed to parse JSON from {path}") from e


def write_json(model: BaseModel, path: Path) -> None:
    try:
        path.write_text(model.model_dump_json(indent=4) + "\n")
    except OSError as e:
        raise RuntimeError(f"Failed to write JSON to {path}") from e


def print_json(model: BaseModel) -> None:
    print(model.model_dump_json(indent=4) + "\n")

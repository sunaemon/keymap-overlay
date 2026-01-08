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
    cols: list[str | None]
    rows: list[str | None]


class SplitConfig(BaseModelAllow):
    enabled: bool = False
    matrix_pins: dict[str, MatrixPins]


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
    usb: USBConfig
    matrix_pins: MatrixPins
    split: SplitConfig | None = None


class QmkKeymapJson(BaseModelAllow):
    version: int | None = None
    # dimension: layer -> flattened index
    layers: list[list[str]] | None = None
    layout: str | None = None


class KeycodesJson(RootModel[dict[str, str]]):
    pass


class CustomKeycodesJson(RootModel[dict[str, str]]):
    pass


class VialMatrix(BaseModelAllow):
    rows: int
    cols: int


class KleKeyProps(BaseModelAllow):
    x: float | None = None
    w: float | None = None
    h: float | None = None

    def has_values(self) -> bool:
        return self.x is not None or self.w is not None or self.h is not None


type KleKey = str | KleKeyProps
type KleRow = list[KleKey]
type KleLayout = list[KleRow]


class VialLayouts(BaseModelAllow):
    keymap: KleLayout


class VialJson(BaseModelAllow):
    name: str
    vendorId: str
    productId: str
    matrix: VialMatrix
    layouts: VialLayouts


class VitalyJson(BaseModelAllow):
    # dimension: layer -> row -> col
    # cf. https://github.com/bskaplou/vitaly/blob/93f08de4b6022007f4e3e655b6d76682e275f4cc/src/protocol.rs#L454
    layout: list[list[list[str]]]


class KeyToLayerJson(RootModel[dict[str, str]]):
    pass


T = TypeVar("T", bound=BaseModel)


class JSONReadError(RuntimeError):
    """Failed to read JSON file."""

    def __init__(self, path: Path, cause: Exception) -> None:
        super().__init__(f"Failed to read JSON from {path}")
        self.__cause__ = cause


class JSONParseError(RuntimeError):
    """Failed to parse JSON content."""

    def __init__(self, path: Path, cause: Exception) -> None:
        super().__init__(f"Failed to parse JSON from {path}")
        self.__cause__ = cause


def parse_json(model: Type[T], path: Path) -> T:
    try:
        return model.model_validate_json(path.read_text())
    except OSError as e:
        raise JSONReadError(path, e) from e
    except Exception as e:
        raise JSONParseError(path, e) from e


def write_json(model: BaseModel, path: Path) -> None:
    tmp_path = path.with_suffix(path.suffix + ".tmp")
    try:
        tmp_path.write_text(model.model_dump_json(indent=4) + "\n")
        tmp_path.replace(path)
    except OSError as e:
        tmp_path.unlink(missing_ok=True)
        raise RuntimeError(f"Failed to write JSON to {path}") from e


def print_json(model: BaseModel, exclude_none: bool = False) -> None:
    print(model.model_dump_json(indent=4, exclude_none=exclude_none) + "\n")

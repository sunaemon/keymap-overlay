# Copyright 2025 sunaemon
# SPDX-License-Identifier: MIT
import re
from pathlib import Path
from typing import Annotated, Type, TypeVar

from pydantic import (
    BaseModel,
    ConfigDict,
    Field,
    RootModel,
    field_validator,
    model_validator,
)


class BaseModelAllow(BaseModel):
    model_config = ConfigDict(extra="allow")


class QmkKeycodeSpecEntry(BaseModelAllow):
    key: str
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

    def layout_keys(self, layout_name: str) -> list[LayoutKey]:
        """Return layout keys for a named layout in keyboard.json."""
        layouts = self.layouts
        if layout_name not in layouts:
            raise ValueError(f"Layout {layout_name} not found in keyboard.json")
        return layouts[layout_name].layout

    def layout_mapping(self, layout_name: str) -> list[tuple[int, int]]:
        """Return (row, col) mapping for a named layout."""
        return [key.matrix for key in self.layout_keys(layout_name)]

    def layout_mapping_dimensions(
        self, layout_name: str
    ) -> tuple[list[tuple[int, int]], int, int]:
        """Return layout mapping and matrix (rows, cols)."""
        mapping = self.layout_mapping(layout_name)
        rows, cols = self.matrix_dimensions()
        return mapping, rows, cols

    def _validate_layout_mapping(
        self,
        mapping: list[tuple[int, int]],
        layout_name: str | None = None,
    ) -> None:
        """Validate layout mapping against matrix dimensions."""
        if not mapping:
            if layout_name:
                raise ValueError(f"Layout {layout_name} mapping is empty")
            raise ValueError("Layout mapping is empty")
        rows, cols = self.matrix_dimensions()
        for r, c in mapping:
            if r >= rows or c >= cols:
                if layout_name:
                    raise ValueError(
                        f"Layout {layout_name} mapping exceeds matrix dimensions"
                    )
                raise ValueError("Layout mapping exceeds matrix dimensions")

    def matrix_rows(self) -> int:
        """Return total matrix rows, including split configuration rows."""
        rows = len(self.matrix_pins.rows)
        if self.split and self.split.enabled:
            if len(self.split.matrix_pins) != 1:
                raise ValueError("multiple split sides not supported yet")
            split_side, matrix_pins = next(iter(self.split.matrix_pins.items()))
            if split_side != "left" and split_side != "right":
                raise ValueError(
                    "only left and right side split configurations are supported yet"
                )
            rows += len(matrix_pins.rows)
        return rows

    def matrix_cols(self) -> int:
        """Return total matrix columns."""
        return len(self.matrix_pins.cols)

    def matrix_dimensions(self) -> tuple[int, int]:
        """Return (rows, cols) for the matrix."""
        return self.matrix_rows(), self.matrix_cols()

    @model_validator(mode="after")
    def _validate_layouts(self) -> "KeyboardJson":
        for name, layout in self.layouts.items():
            mapping = [key.matrix for key in layout.layout]
            self._validate_layout_mapping(mapping, layout_name=name)
        return self


class QmkKeymapJson(BaseModelAllow):
    version: int | None = None
    # dimension: layer -> flattened index
    layers: list[list[str]]
    layout: str | None = None


HEX_KEY_RE = re.compile(r"0x[0-9A-Fa-f]{1,4}")


def _validate_hex_map(v: dict[str, str]) -> dict[str, str]:
    bad = [k for k in v if not HEX_KEY_RE.fullmatch(k)]
    if bad:
        raise ValueError(f"invalid keys: {bad}")
    return v


class KeycodesJson(RootModel[dict[str, str]]):
    @field_validator("root", mode="before")
    @classmethod
    def parse_hex_map(cls, v: dict[str, str]) -> dict[str, str]:
        return _validate_hex_map(v)


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


T = TypeVar("T", bound=BaseModel)


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

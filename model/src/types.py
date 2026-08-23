# Copyright 2025 sunaemon
# SPDX-License-Identifier: MIT
import re
import sys
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
    keycodes: Annotated[dict[str, QmkKeycodeSpecEntry], Field()]


class LayoutKey(BaseModelAllow):
    x: float
    y: float
    matrix: Annotated[tuple[int, int], Field(description="Row, Column")]
    w: float = 1.0
    h: float = 1.0
    label: str | None = None
    r: float = 0.0
    rx: float | None = None
    ry: float | None = None


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


class RotaryEncoder(BaseModelAllow):
    """Describes the pins connected to a rotary encoder."""

    pin_a: str
    pin_b: str


class EncoderConfig(BaseModelAllow):
    """Configures the keyboard's rotary encoders."""

    rotary: list[RotaryEncoder]


class EncoderPlacement(BaseModel):
    """Places one encoder at a key matrix position or explicit coordinates."""

    matrix: tuple[int, int] | None = None
    x: float | None = None
    y: float | None = None

    @model_validator(mode="after")
    def _validate_position(self) -> "EncoderPlacement":
        has_coordinates = self.x is not None or self.y is not None
        if self.matrix is not None and has_coordinates:
            raise ValueError("encoder placement cannot mix matrix and coordinates")
        if self.matrix is None and (self.x is None or self.y is None):
            raise ValueError("encoder placement requires matrix or both x and y")
        return self


class KeyboardConfig(BaseModel):
    """Project-specific keyboard settings outside QMK's schema."""

    qmk_keyboard: str
    encoders: list[EncoderPlacement] = Field(default_factory=list)


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
    encoder: EncoderConfig | None = None

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

    def encoder_count(self) -> int:
        """Return the number of rotary encoders."""
        if self.encoder is None:
            return 0
        return len(self.encoder.rotary)

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
            if r < 0 or c < 0:
                if layout_name:
                    raise ValueError(
                        f"Layout {layout_name} mapping contains negative indices"
                    )
                raise ValueError("Layout mapping contains negative indices")
            if r >= rows or c >= cols:
                if layout_name:
                    raise ValueError(
                        f"Layout {layout_name} mapping exceeds matrix dimensions"
                    )
                raise ValueError("Layout mapping exceeds matrix dimensions")

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


class KeycodesJson(RootModel[dict[str, str]]):
    @field_validator("root", mode="before")
    @classmethod
    def parse_hex_map(cls, v: dict[str, str]) -> dict[str, str]:
        return cls._validate_hex_map(v)

    @staticmethod
    def _validate_hex_map(v: dict[str, str]) -> dict[str, str]:
        bad = [k for k in v if not HEX_KEY_RE.fullmatch(k)]
        if bad:
            raise ValueError(f"invalid keys: {bad}")
        return v


class VialMatrix(BaseModelAllow):
    rows: int
    cols: int


class KleKeyProps(BaseModelAllow):
    x: float | None = None
    y: float | None = None
    w: float | None = None
    h: float | None = None

    def has_values(self) -> bool:
        """Return True if any position/size property is set."""
        return (
            self.x is not None
            or self.y is not None
            or self.w is not None
            or self.h is not None
        )


type KleKey = str | KleKeyProps
type KleRow = list[KleKey]
type KleLayout = list[KleRow]


class VialLayouts(BaseModelAllow):
    keymap: KleLayout


class VialCustomKeycode(BaseModelAllow):
    """One QK_KB_0-based custom keycode, in enum declaration order."""

    name: str
    title: str = ""
    shortName: str = ""


class KeymapOverlayMetadata(BaseModel):
    """Metadata that lets the runtime render this keyboard without host config."""

    keyboardId: int
    layoutName: str
    pixelsPerUnit: int
    keyboard: KeyboardJson
    config: KeyboardConfig


class VialJson(BaseModelAllow):
    name: str
    vendorId: str
    productId: str
    matrix: VialMatrix
    layouts: VialLayouts
    customKeycodes: list[VialCustomKeycode] | None = None
    keymapOverlay: KeymapOverlayMetadata | None = None


class VitalyJson(BaseModelAllow):
    # dimension: layer -> row -> col
    # cf. https://github.com/bskaplou/vitaly/blob/93f08de4b6022007f4e3e655b6d76682e275f4cc/src/protocol.rs#L454
    layout: list[list[list[str]]]
    # dimension: layer -> encoder -> direction (counter-clockwise, clockwise)
    encoder_layout: list[list[list[str]]] | None = None


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
        # Explicit encoding: read_text otherwise decodes with the locale's
        # codepage, which on a Japanese Windows install is cp932 and mangles
        # every non-ASCII keymap this reads.
        return model.model_validate_json(path.read_text(encoding="utf-8"))
    except OSError as e:
        raise JSONReadError(path, e) from e
    except Exception as e:
        raise JSONParseError(path, e) from e


def print_json(model: BaseModel, exclude_none: bool = False) -> None:
    """Writes the model to stdout as UTF-8 JSON."""
    # The counterpart to parse_json's explicit encoding. model_dump_json emits
    # real non-ASCII characters rather than \\u escapes, so printing through a
    # cp932 stdout raises UnicodeEncodeError and WRITE_OUTPUT's redirect leaves
    # no file at all. Writing encoded bytes bypasses the locale codepage.
    text = model.model_dump_json(indent=4, exclude_none=exclude_none) + "\n\n"
    buffer = getattr(sys.stdout, "buffer", None)
    if buffer is None:
        # A text-only stream — io.StringIO, or pytest's capsys — has no binary
        # buffer and applies no encoding of its own, so the text goes straight
        # out.
        sys.stdout.write(text)
        sys.stdout.flush()
    else:
        # Flush first, so anything already written through the text layer stays
        # ahead of these bytes rather than trailing them.
        sys.stdout.flush()
        buffer.write(text.encode("utf-8"))
        buffer.flush()

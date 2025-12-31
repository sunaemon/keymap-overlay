# Copyright 2025 sunaemon
# SPDX-License-Identifier: MIT
from pydantic import BaseModel, ConfigDict, Field, RootModel


class BaseModelAllow(BaseModel):
    model_config = ConfigDict(extra="allow")


class QmkKeycodeSpecEntry(BaseModelAllow):
    key: str | None = None
    aliases: list[str] | None = None


class QmkKeycodesSpec(BaseModelAllow):
    keycodes: dict[str, QmkKeycodeSpecEntry] = Field(default_factory=dict)


class LayoutKey(BaseModelAllow):
    x: float
    y: float
    matrix: tuple[int, int] | None = None
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


class QmkKeyboardJson(BaseModelAllow):
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
    layers: list[list[str | int]] | None = None
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
    layers: list[list[list[str | int]]] | None = None
    layout: list[list[list[str | int]]] | None = None


class KeyToLayerJson(RootModel[dict[str, str]]):
    pass

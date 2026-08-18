# Copyright 2026 sunaemon
# SPDX-License-Identifier: MIT
import json
import lzma

import pytest

from model.scripts import fetch_vial_definition as module

DEFINITION = {
    "name": "Test",
    "vendorId": "0xFEED",
    "productId": "0x0000",
    "matrix": {"rows": 1, "cols": 1},
    "layouts": {"keymap": [["0,0"]]},
    "customKeycodes": [{"name": "KC_ALPHA", "title": "", "shortName": "α"}],
}


class FakeRawHidDevice:
    """Answers Vial's get-size/get-definition exchange from an in-memory blob."""

    def __init__(self, compressed: bytes) -> None:
        self._compressed = compressed
        self._pending = bytes(module.REPORT_LENGTH)
        self.closed = False

    def open_path(self, path: bytes) -> None:
        self.path = path

    def close(self) -> None:
        self.closed = True

    def write(self, report: bytes) -> int:
        payload = bytes(report[1:])
        if payload[:2] == bytes([module.VIAL_PREFIX, module.VIAL_GET_SIZE]):
            self._pending = self._pad(
                len(self._compressed).to_bytes(4, byteorder="little")
            )
        elif payload[:2] == bytes([module.VIAL_PREFIX, module.VIAL_GET_DEFINITION]):
            block_index = int.from_bytes(payload[2:6], byteorder="little")
            start = block_index * module.REPORT_LENGTH
            self._pending = self._pad(
                self._compressed[start : start + module.REPORT_LENGTH]
            )
        return len(report)

    def read(self, max_length: int, timeout_ms: int = 0) -> list[int]:
        return list(self._pending[:max_length])

    def _pad(self, chunk: bytes) -> bytes:
        return chunk + bytes(module.REPORT_LENGTH - len(chunk))


def test_fetch_vial_definition_reassembles_and_decompresses_the_device_blob(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    """The definition spans several 32-byte reports and must be reassembled in order."""
    compressed = lzma.compress(
        json.dumps(DEFINITION).encode("utf-8"), format=lzma.FORMAT_XZ
    )
    fake_device = FakeRawHidDevice(compressed)

    def fake_enumerate(vendor_id: int, product_id: int) -> list[dict]:
        assert (vendor_id, product_id) == (0xFEED, 0x0000)
        return [
            {"usage_page": 0x0001, "usage": 0x06, "path": b"boot-keyboard"},
            {
                "usage_page": module.RAW_USAGE_PAGE,
                "usage": 0x99,
                "path": b"wrong-usage",
            },
            {
                "usage_page": module.RAW_USAGE_PAGE,
                "usage": module.RAW_USAGE_ID,
                "path": b"raw-hid",
            },
        ]

    monkeypatch.setattr(module.hid, "enumerate", fake_enumerate)
    monkeypatch.setattr(module.hid, "device", lambda: fake_device)

    definition = module.fetch_vial_definition("0xFEED", "0x0000")

    assert fake_device.path == b"raw-hid"
    assert fake_device.closed
    assert definition.name == "Test"
    custom_keycodes = definition.customKeycodes
    assert custom_keycodes is not None
    assert custom_keycodes[0].name == "KC_ALPHA"
    assert custom_keycodes[0].shortName == "α"


def test_no_matching_raw_hid_interface_is_rejected(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setattr(module.hid, "enumerate", lambda vendor_id, product_id: [])

    with pytest.raises(ValueError, match="No Raw HID interface found"):
        module.fetch_vial_definition("0xFEED", "0x0000")

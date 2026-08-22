# Copyright 2026 sunaemon
# SPDX-License-Identifier: MIT
import logging
import lzma
from pathlib import Path
from typing import Annotated, Protocol

import hid
import typer

from model.src.types import KeyboardJson, VialJson, parse_json, print_json
from model.src.util import initialize_logging, parse_hex_keycode

logger = logging.getLogger(__name__)

app = typer.Typer()

# Must match RAW_USAGE_PAGE/RAW_USAGE_ID in
# overlay/keymap-overlay-runtime/src/lib.rs: a QMK keyboard exposes several
# HID interfaces at the same vendor/product id, and only this one is Raw HID.
RAW_USAGE_PAGE = 0xFF60
RAW_USAGE_ID = 0x61

# Vial's report size and command bytes, cf. vitaly's MESSAGE_LENGTH and
# CMD_VIA_VIAL_PREFIX/CMD_VIAL_GET_SIZE/CMD_VIAL_GET_DEFINITION (protocol.rs).
REPORT_LENGTH = 32
MAX_DEFINITION_SIZE = 16 * 1024 * 1024
VIAL_PREFIX = 0xFE
VIAL_GET_SIZE = 0x01
VIAL_GET_DEFINITION = 0x02
READ_TIMEOUT_MS = 500


class RawHidTransport(Protocol):
    """Provides the Raw HID operations used by the Vial exchange."""

    def write(self, data: bytes) -> int: ...

    def read(self, max_length: int, timeout_ms: int = 0) -> list[int]: ...


@app.command()
def main(
    keyboard_json: Annotated[
        Path, typer.Option(help="QMK keyboard.json, for the device's vendor/product id")
    ],
) -> None:
    """Fetch the connected keyboard's embedded Vial definition and emit it to stdout."""
    initialize_logging()
    try:
        keyboard = parse_json(KeyboardJson, keyboard_json)
        definition = fetch_vial_definition(keyboard.usb.vid, keyboard.usb.pid)
        print_json(definition, exclude_none=True)
        logger.info("Fetched Vial definition for %s", keyboard.keyboard_name)
    except Exception:
        logger.exception("Failed to fetch Vial definition from the connected device")
        raise typer.Exit(code=1) from None


def fetch_vial_definition(vendor_id: str, product_id: str) -> VialJson:
    """Read, decompress, and parse the device's embedded Vial definition."""
    device = _open_raw_hid_device(vendor_id, product_id)
    try:
        compressed = _read_definition(device)
    finally:
        device.close()
    return VialJson.model_validate_json(lzma.decompress(compressed))


def _open_raw_hid_device(vendor_id: str, product_id: str) -> hid.device:
    numeric_vendor_id = parse_hex_keycode(vendor_id)
    numeric_product_id = parse_hex_keycode(product_id)
    if numeric_vendor_id is None or numeric_product_id is None:
        raise ValueError(f"Invalid vendor/product id: {vendor_id}/{product_id}")

    for info in hid.enumerate(numeric_vendor_id, numeric_product_id):
        if info["usage_page"] == RAW_USAGE_PAGE and info["usage"] == RAW_USAGE_ID:
            device = hid.device()
            device.open_path(info["path"])
            return device
    raise ValueError(f"No Raw HID interface found for device {vendor_id}:{product_id}")


def _read_definition(device: RawHidTransport) -> bytes:
    remaining = _read_definition_size(device)
    if not 0 < remaining <= MAX_DEFINITION_SIZE:
        raise ValueError(f"Invalid Vial definition size: {remaining}")
    blocks = bytearray()
    block_index = 0
    while len(blocks) < remaining:
        response = _send_recv(
            device,
            [VIAL_PREFIX, VIAL_GET_DEFINITION, *_block_index_bytes(block_index)],
        )
        if len(response) != REPORT_LENGTH:
            raise OSError(
                f"Invalid Vial definition reply length: {len(response)}; expected {REPORT_LENGTH}"
            )
        blocks.extend(response)
        block_index += 1
    return bytes(blocks[:remaining])


def _read_definition_size(device: RawHidTransport) -> int:
    response = _send_recv(device, [VIAL_PREFIX, VIAL_GET_SIZE])
    if len(response) != REPORT_LENGTH:
        raise OSError(
            f"Invalid Vial size reply length: {len(response)}; expected {REPORT_LENGTH}"
        )
    return int.from_bytes(response[:4], byteorder="little")


def _block_index_bytes(block_index: int) -> list[int]:
    return list(block_index.to_bytes(4, byteorder="little"))


def _send_recv(device: RawHidTransport, payload: list[int]) -> bytes:
    """Sends one report (report id 0) and reads the matching reply."""
    report = bytes([0x00, *payload, *([0] * (REPORT_LENGTH - len(payload)))])
    device.write(report)
    return bytes(device.read(REPORT_LENGTH, READ_TIMEOUT_MS))


if __name__ == "__main__":
    app()

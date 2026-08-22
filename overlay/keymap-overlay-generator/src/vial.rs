//! Minimal read-only Vial protocol support used by the live overlay model.

use anyhow::{Context, Result, bail};
use hidapi::HidDevice;
use lzma_rust2::XzReader;
use serde_json::Value;
use std::cmp::min;
use std::io::Read;

pub const USAGE_PAGE: u16 = 0xFF60;
pub const USAGE_ID: u16 = 0x61;

const MESSAGE_LENGTH: usize = 32;
const VIA_UNHANDLED: u8 = 0xFF;
const CMD_VIA_GET_PROTOCOL_VERSION: u8 = 0x01;
const CMD_VIA_VIAL_PREFIX: u8 = 0xFE;
const CMD_VIA_GET_LAYER_COUNT: u8 = 0x11;
const CMD_VIA_KEYMAP_GET_BUFFER: u8 = 0x12;
const CMD_VIAL_GET_KEYBOARD_ID: u8 = 0x00;
const CMD_VIAL_GET_SIZE: u8 = 0x01;
const CMD_VIAL_GET_DEFINITION: u8 = 0x02;
const CMD_VIAL_GET_ENCODER: u8 = 0x03;
const BUFFER_FETCH_CHUNK: u16 = 28;

/// The Vial values needed to render a device-owned keymap.
pub struct DeviceModel {
    pub layer_count: u8,
    pub matrix_rows: u8,
    pub matrix_cols: u8,
    pub custom_keycodes: Value,
    pub keycodes: Vec<u16>,
    pub encoders: Vec<Vec<[u16; 2]>>,
}

/// Reads one device's Vial metadata, dynamic keymap and encoder bindings.
pub fn read_device_model(device: &HidDevice, encoder_count: usize) -> Result<DeviceModel> {
    let _via_version = send_recv(device, &[CMD_VIA_GET_PROTOCOL_VERSION])?[2];
    let keyboard_id = send_recv(device, &[CMD_VIA_VIAL_PREFIX, CMD_VIAL_GET_KEYBOARD_ID])?;
    if keyboard_id[0] == VIA_UNHANDLED {
        bail!("Connected device does not implement the Vial protocol");
    }
    let layer_count = send_recv(device, &[CMD_VIA_GET_LAYER_COUNT])?[1];
    if layer_count == 0 {
        bail!("Device reports zero Vial layers");
    }
    let definition = read_definition(device)?;
    let matrix = definition
        .get("matrix")
        .context("matrix missing from the device's Vial definition")?;
    let matrix_rows = matrix["rows"]
        .as_u64()
        .context("matrix/rows missing from the device's Vial definition")?
        as u8;
    let matrix_cols = matrix["cols"]
        .as_u64()
        .context("matrix/cols missing from the device's Vial definition")?
        as u8;
    let keycodes = read_keycodes(device, layer_count, matrix_rows, matrix_cols)?;
    let encoders = (0..layer_count)
        .map(|layer| read_encoders(device, layer, encoder_count))
        .collect::<Result<_>>()?;

    Ok(DeviceModel {
        layer_count,
        matrix_rows,
        matrix_cols,
        custom_keycodes: definition["customKeycodes"].clone(),
        keycodes,
        encoders,
    })
}

fn read_definition(device: &HidDevice) -> Result<Value> {
    let size = send_recv(device, &[CMD_VIA_VIAL_PREFIX, CMD_VIAL_GET_SIZE])?;
    if size[0] == VIA_UNHANDLED {
        bail!("Device does not expose a Vial definition");
    }
    let size = u32::from_le_bytes([size[0], size[1], size[2], size[3]]) as usize;
    let mut compressed = Vec::with_capacity(size);
    for block in 0..size.div_ceil(MESSAGE_LENGTH) {
        let response = send_recv(
            device,
            &[
                CMD_VIA_VIAL_PREFIX,
                CMD_VIAL_GET_DEFINITION,
                block as u8,
                (block >> 8) as u8,
                (block >> 16) as u8,
                (block >> 24) as u8,
            ],
        )?;
        let remaining = size.saturating_sub(compressed.len());
        compressed.extend_from_slice(&response[..min(remaining, MESSAGE_LENGTH)]);
    }
    let mut decoded = Vec::new();
    XzReader::new(compressed.as_slice(), true)
        .read_to_end(&mut decoded)
        .context("Failed to decompress the device's Vial definition")?;
    serde_json::from_slice(&decoded).context("Failed to parse the device's Vial definition")
}

fn read_keycodes(device: &HidDevice, layers: u8, rows: u8, cols: u8) -> Result<Vec<u16>> {
    let size = layers as u16 * rows as u16 * cols as u16 * 2;
    let mut bytes = Vec::with_capacity(size as usize);
    let mut offset = 0;
    while offset < size {
        let chunk = min(size - offset, BUFFER_FETCH_CHUNK);
        let response = send_recv(
            device,
            &[
                CMD_VIA_KEYMAP_GET_BUFFER,
                (offset >> 8) as u8,
                offset as u8,
                chunk as u8,
            ],
        )?;
        if response[0] == VIA_UNHANDLED {
            bail!("Device rejected Vial keymap read at byte offset {offset}");
        }
        bytes.extend_from_slice(&response[4..4 + chunk as usize]);
        offset += chunk;
    }
    Ok(bytes
        .chunks_exact(2)
        .map(|bytes| u16::from_be_bytes([bytes[0], bytes[1]]))
        .collect())
}

fn read_encoders(device: &HidDevice, layer: u8, count: usize) -> Result<Vec<[u16; 2]>> {
    (0..count)
        .map(|index| {
            let response = send_recv(
                device,
                &[
                    CMD_VIA_VIAL_PREFIX,
                    CMD_VIAL_GET_ENCODER,
                    layer,
                    index as u8,
                ],
            )?;
            Ok([
                u16::from_be_bytes([response[0], response[1]]),
                u16::from_be_bytes([response[2], response[3]]),
            ])
        })
        .collect()
}

fn send_recv(device: &HidDevice, request: &[u8]) -> Result<[u8; MESSAGE_LENGTH]> {
    let mut report = [0; MESSAGE_LENGTH + 1];
    report[1..request.len() + 1].copy_from_slice(request);
    device
        .write(&report)
        .context("Failed to send a Vial request")?;
    let mut response = [0; MESSAGE_LENGTH];
    device
        .read_timeout(&mut response, 500)
        .context("Timed out waiting for a Vial response")?;
    Ok(response)
}

#[cfg(test)]
mod tests {
    #[test]
    fn decodes_vial_keymap_bytes_as_big_endian_keycodes() {
        let keycodes = vec![0x00, 0x04, 0x52, 0x21];
        assert_eq!(
            keycodes
                .chunks_exact(2)
                .map(|bytes| u16::from_be_bytes([bytes[0], bytes[1]]))
                .collect::<Vec<_>>(),
            vec![0x0004, 0x5221]
        );
    }
}

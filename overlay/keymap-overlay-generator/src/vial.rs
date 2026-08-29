//! Minimal read-only Vial protocol support used by the live overlay model.

use anyhow::{Context, Result, bail};
use hidapi::HidDevice;
use keymap_core::{RAW_HID_REPORT_MAGIC, RawLayerEvent, parse_raw_layer_event};
use lzma_rust2::XzReader;
use serde_json::Value;
use std::cmp::min;
use std::io::Read;
use std::time::{Duration, Instant};

pub const USAGE_PAGE: u16 = 0xFF60;
pub const USAGE_ID: u16 = 0x61;

const MESSAGE_LENGTH: usize = 32;
const MAX_INPUT_MESSAGE_LENGTH: usize = MESSAGE_LENGTH + 1;
const VIA_UNHANDLED: u8 = 0xFF;
const CMD_VIA_GET_PROTOCOL_VERSION: u8 = 0x01;
const CMD_VIA_VIAL_PREFIX: u8 = 0xFE;
const CMD_VIA_GET_LAYER_COUNT: u8 = 0x11;
const CMD_VIA_KEYMAP_GET_BUFFER: u8 = 0x12;
const CMD_VIAL_GET_KEYBOARD_ID: u8 = 0x00;
const CMD_VIAL_GET_SIZE: u8 = 0x01;
const CMD_VIAL_GET_DEFINITION: u8 = 0x02;
const CMD_VIAL_GET_ENCODER: u8 = 0x03;
const BUFFER_FETCH_CHUNK: usize = 28;
const RESPONSE_TIMEOUT: Duration = Duration::from_millis(500);
// Definitions are normally a few KiB. Limits keep a malformed HID device from
// forcing an unbounded allocation or XZ decompression while the overlay starts.
const MAX_COMPRESSED_DEFINITION_BYTES: usize = 1_048_576;
const MAX_DECODED_DEFINITION_BYTES: usize = 4_194_304;

/// The Vial values needed to render a device-owned keymap.
pub struct DeviceModel {
    pub layer_count: u8,
    pub matrix_rows: u8,
    pub matrix_cols: u8,
    pub vial_definition: Value,
    pub keycodes: Vec<u16>,
    pub encoders: Vec<Vec<[u16; 2]>>,
}

/// Reads one device's Vial metadata, dynamic keymap and encoder bindings.
pub fn read_device_model(device: &HidDevice, encoder_count: usize) -> Result<DeviceModel> {
    let mut layer_events = Vec::new();
    read_device_model_recording_events(device, encoder_count, &mut layer_events)
}

/// Reads and validates the Vial definition embedded in one device.
pub fn read_device_definition(device: &HidDevice) -> Result<Value> {
    let mut layer_events = Vec::new();
    read_device_definition_recording_events(device, &mut layer_events)
}

/// Reads dynamic Vial state using an already-fetched embedded definition.
pub fn read_device_model_with_definition(
    device: &HidDevice,
    definition: Value,
    encoder_count: usize,
) -> Result<DeviceModel> {
    let mut layer_events = Vec::new();
    read_device_model_with_definition_recording_events(
        device,
        definition,
        encoder_count,
        &mut layer_events,
    )
}

pub(crate) fn read_device_definition_recording_events(
    device: &HidDevice,
    layer_events: &mut Vec<RawLayerEvent>,
) -> Result<Value> {
    let via_request = [CMD_VIA_GET_PROTOCOL_VERSION];
    let via_version = send_recv(device, &via_request, layer_events)?;
    if is_unhandled_response(&via_version, &via_request) {
        bail!("Connected device does not implement the VIA protocol");
    }
    let keyboard_id_request = [CMD_VIA_VIAL_PREFIX, CMD_VIAL_GET_KEYBOARD_ID];
    let keyboard_id = send_recv(device, &keyboard_id_request, layer_events)?;
    if is_unhandled_response(&keyboard_id, &keyboard_id_request) {
        bail!("Connected device does not implement the Vial protocol");
    }
    read_definition(device, layer_events)
}

pub(crate) fn read_device_model_with_definition_recording_events(
    device: &HidDevice,
    definition: Value,
    encoder_count: usize,
    layer_events: &mut Vec<RawLayerEvent>,
) -> Result<DeviceModel> {
    let layer_count_request = [CMD_VIA_GET_LAYER_COUNT];
    let layer_count_response = send_recv(device, &layer_count_request, layer_events)?;
    if is_unhandled_response(&layer_count_response, &layer_count_request) {
        bail!("Device does not expose a Vial layer count");
    }
    let layer_count = layer_count_response[1];
    if layer_count == 0 {
        bail!("Device reports zero Vial layers");
    }
    let matrix = definition
        .get("matrix")
        .context("matrix missing from the device's Vial definition")?;
    let matrix_rows = matrix_dimension(matrix, "rows")?;
    let matrix_cols = matrix_dimension(matrix, "cols")?;
    if encoder_count > usize::from(u8::MAX) + 1 {
        bail!("Device configuration has too many encoders for the Vial protocol");
    }
    let keycodes = read_keycodes(device, layer_count, matrix_rows, matrix_cols, layer_events)?;
    let encoders = (0..layer_count)
        .map(|layer| read_encoders(device, layer, encoder_count, layer_events))
        .collect::<Result<_>>()?;

    Ok(DeviceModel {
        layer_count,
        matrix_rows,
        matrix_cols,
        vial_definition: definition,
        keycodes,
        encoders,
    })
}

fn read_device_model_recording_events(
    device: &HidDevice,
    encoder_count: usize,
    layer_events: &mut Vec<RawLayerEvent>,
) -> Result<DeviceModel> {
    let definition = read_device_definition_recording_events(device, layer_events)?;
    read_device_model_with_definition_recording_events(
        device,
        definition,
        encoder_count,
        layer_events,
    )
}

fn matrix_dimension(matrix: &Value, name: &str) -> Result<u8> {
    let dimension = matrix[name]
        .as_u64()
        .with_context(|| format!("matrix/{name} missing from the device's Vial definition"))?;
    u8::try_from(dimension).with_context(|| format!("matrix/{name} exceeds Vial's supported range"))
}

fn read_definition(device: &HidDevice, layer_events: &mut Vec<RawLayerEvent>) -> Result<Value> {
    // This Vial command returns a raw little-endian size with no status byte.
    // A valid size can therefore begin with 0xFF; the keyboard-id handshake
    // above is what establishes that the device supports Vial.
    let size_response = send_recv(
        device,
        &[CMD_VIA_VIAL_PREFIX, CMD_VIAL_GET_SIZE],
        layer_events,
    )?;
    let size = definition_size(&size_response);
    if size == 0 {
        bail!("Device reports an empty Vial definition");
    }
    if size > MAX_COMPRESSED_DEFINITION_BYTES {
        bail!(
            "Device Vial definition is {size} bytes, exceeding the {MAX_COMPRESSED_DEFINITION_BYTES}-byte compressed limit"
        );
    }
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
            layer_events,
        )?;
        let remaining = size.saturating_sub(compressed.len());
        compressed.extend_from_slice(&response[..min(remaining, MESSAGE_LENGTH)]);
    }
    let mut decoded = Vec::new();
    let mut reader = XzReader::new(compressed.as_slice(), true);
    reader
        .by_ref()
        .take((MAX_DECODED_DEFINITION_BYTES + 1) as u64)
        .read_to_end(&mut decoded)
        .context("Failed to decompress the device's Vial definition")?;
    if decoded.len() > MAX_DECODED_DEFINITION_BYTES {
        bail!(
            "Device Vial definition exceeds the {MAX_DECODED_DEFINITION_BYTES}-byte decoded limit"
        );
    }
    serde_json::from_slice(&decoded).context("Failed to parse the device's Vial definition")
}

fn read_keycodes(
    device: &HidDevice,
    layers: u8,
    rows: u8,
    cols: u8,
    layer_events: &mut Vec<RawLayerEvent>,
) -> Result<Vec<u16>> {
    let size = keymap_byte_len(layers, rows, cols)?;
    let mut bytes = Vec::with_capacity(size);
    let mut offset = 0_usize;
    while offset < size {
        let chunk = min(size - offset, BUFFER_FETCH_CHUNK);
        let request = [
            CMD_VIA_KEYMAP_GET_BUFFER,
            (offset >> 8) as u8,
            offset as u8,
            chunk as u8,
        ];
        let response = send_recv(device, &request, layer_events)?;
        if is_unhandled_response(&response, &request) {
            bail!("Device rejected Vial keymap read at byte offset {offset}");
        }
        bytes.extend_from_slice(&response[4..4 + chunk]);
        offset += chunk;
    }
    Ok(bytes
        .chunks_exact(2)
        .map(|bytes| u16::from_be_bytes([bytes[0], bytes[1]]))
        .collect())
}

fn keymap_byte_len(layers: u8, rows: u8, cols: u8) -> Result<usize> {
    let byte_len = usize::from(layers)
        .checked_mul(usize::from(rows))
        .and_then(|size| size.checked_mul(usize::from(cols)))
        .and_then(|size| size.checked_mul(2))
        .context("Device Vial keymap dimensions overflow")?;
    if byte_len > usize::from(u16::MAX) {
        bail!("Device Vial keymap is {byte_len} bytes, exceeding the 16-bit buffer range");
    }
    Ok(byte_len)
}

fn read_encoders(
    device: &HidDevice,
    layer: u8,
    count: usize,
    layer_events: &mut Vec<RawLayerEvent>,
) -> Result<Vec<[u16; 2]>> {
    (0..count)
        .map(|index| {
            let request = [
                CMD_VIA_VIAL_PREFIX,
                CMD_VIAL_GET_ENCODER,
                layer,
                index as u8,
            ];
            let response = send_recv(device, &request, layer_events)?;
            if is_unhandled_response(&response, &request)
                || response_matches_request(&response, &request)
            {
                bail!("Device rejected Vial encoder read for layer {layer}, encoder {index}");
            }
            Ok(decode_encoder_response(&response))
        })
        .collect()
}

fn definition_size(response: &[u8; MESSAGE_LENGTH]) -> usize {
    u32::from_le_bytes([response[0], response[1], response[2], response[3]]) as usize
}

fn decode_encoder_response(response: &[u8; MESSAGE_LENGTH]) -> [u16; 2] {
    [
        u16::from_be_bytes([response[0], response[1]]),
        u16::from_be_bytes([response[2], response[3]]),
    ]
}

fn is_unhandled_response(response: &[u8; MESSAGE_LENGTH], request: &[u8]) -> bool {
    response[0] == VIA_UNHANDLED
        && response[1..request.len()] == request[1..]
        && response[request.len()..].iter().all(|byte| *byte == 0)
}

fn response_matches_request(response: &[u8; MESSAGE_LENGTH], request: &[u8]) -> bool {
    response[..request.len()] == *request && response[request.len()..].iter().all(|byte| *byte == 0)
}

fn send_recv(
    device: &HidDevice,
    request: &[u8],
    layer_events: &mut Vec<RawLayerEvent>,
) -> Result<[u8; MESSAGE_LENGTH]> {
    if request.len() > MESSAGE_LENGTH {
        bail!("Vial request exceeds the {MESSAGE_LENGTH}-byte HID report size");
    }
    let mut report = [0; MESSAGE_LENGTH + 1];
    report[1..request.len() + 1].copy_from_slice(request);
    device
        .write(&report)
        .context("Failed to send a Vial request")?;

    let deadline = Instant::now() + RESPONSE_TIMEOUT;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            bail!("Timed out waiting for a Vial response");
        }
        let timeout_ms = i32::try_from(remaining.as_millis().max(1)).unwrap_or(i32::MAX);
        let mut response = [0; MAX_INPUT_MESSAGE_LENGTH];
        let response_len = device
            .read_timeout(&mut response, timeout_ms)
            .context("Failed while waiting for a Vial response")?;
        if let Some(response) = classify_vial_response(response, response_len, layer_events)? {
            return Ok(response);
        }
    }
}

fn classify_vial_response(
    response: [u8; MAX_INPUT_MESSAGE_LENGTH],
    response_len: usize,
    layer_events: &mut Vec<RawLayerEvent>,
) -> Result<Option<[u8; MESSAGE_LENGTH]>> {
    if response_len == 0 {
        bail!("Timed out waiting for a Vial response");
    }
    let response = normalize_input_report(response, response_len)?;
    if response.starts_with(&RAW_HID_REPORT_MAGIC)
        && matches!(response[6], 0 | 1)
        && response[7..].iter().all(|byte| *byte == 0)
    {
        if let Some(event) = parse_raw_layer_event(&response) {
            layer_events.push(event);
        }
        return Ok(None);
    }
    Ok(Some(response))
}

fn normalize_input_report(
    report: [u8; MAX_INPUT_MESSAGE_LENGTH],
    report_len: usize,
) -> Result<[u8; MESSAGE_LENGTH]> {
    let payload = match report_len {
        MESSAGE_LENGTH => &report[..MESSAGE_LENGTH],
        MAX_INPUT_MESSAGE_LENGTH => &report[1..MAX_INPUT_MESSAGE_LENGTH],
        _ => {
            bail!(
                "Incomplete Vial response: expected {MESSAGE_LENGTH} or {MAX_INPUT_MESSAGE_LENGTH} bytes, received {report_len}"
            )
        }
    };
    let mut normalized = [0; MESSAGE_LENGTH];
    normalized.copy_from_slice(payload);
    Ok(normalized)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn decodes_vial_keymap_bytes_as_big_endian_keycodes() {
        let keycodes = [0x00, 0x04, 0x52, 0x21];
        assert_eq!(
            keycodes
                .chunks_exact(2)
                .map(|bytes| u16::from_be_bytes([bytes[0], bytes[1]]))
                .collect::<Vec<_>>(),
            vec![0x0004, 0x5221]
        );
    }

    #[test]
    fn rejects_matrix_dimensions_outside_the_vial_byte_range() {
        let matrix = json!({ "rows": 256 });
        assert!(matrix_dimension(&matrix, "rows").is_err());
    }

    #[test]
    fn rejects_keymaps_that_exceed_the_vial_buffer_range() {
        assert!(keymap_byte_len(u8::MAX, u8::MAX, u8::MAX).is_err());
    }

    #[test]
    fn definition_size_can_legitimately_start_with_the_unhandled_byte() {
        let mut response = [0; MESSAGE_LENGTH];
        response[..4].copy_from_slice(&[0xFF, 0x01, 0x00, 0x00]);
        assert_eq!(definition_size(&response), 511);
    }

    #[test]
    fn encoder_keycodes_can_legitimately_start_with_the_unhandled_byte() {
        let mut response = [0; MESSAGE_LENGTH];
        response[..4].copy_from_slice(&[0xFF, 0x01, 0xFF, 0x02]);
        assert_eq!(decode_encoder_response(&response), [0xFF01, 0xFF02]);
    }

    #[test]
    fn distinguishes_an_unhandled_response_from_a_valid_ff_payload() {
        let request = [CMD_VIA_VIAL_PREFIX, CMD_VIAL_GET_ENCODER, 2, 1];
        let mut unhandled = [0; MESSAGE_LENGTH];
        unhandled[..request.len()].copy_from_slice(&request);
        unhandled[0] = VIA_UNHANDLED;
        assert!(is_unhandled_response(&unhandled, &request));

        let mut payload = [0; MESSAGE_LENGTH];
        payload[..4].copy_from_slice(&[0xFF, 0x01, 0xFF, 0x02]);
        assert!(!is_unhandled_response(&payload, &request));
    }

    #[test]
    fn detects_an_encoder_request_the_firmware_left_unchanged() {
        let request = [CMD_VIA_VIAL_PREFIX, CMD_VIAL_GET_ENCODER, 2, 1];
        let mut response = [0; MESSAGE_LENGTH];
        response[..request.len()].copy_from_slice(&request);
        assert!(response_matches_request(&response, &request));
    }

    #[test]
    fn preserves_unsolicited_layer_events_while_waiting_for_vial() {
        let mut layer_event = [0; MAX_INPUT_MESSAGE_LENGTH];
        layer_event[..7].copy_from_slice(&[b'K', b'M', b'O', 1, 2, 3, 1]);
        let mut layer_events = Vec::new();

        assert_eq!(
            classify_vial_response(layer_event, MESSAGE_LENGTH, &mut layer_events).unwrap(),
            None
        );
        assert_eq!(
            layer_events,
            vec![RawLayerEvent {
                keyboard_id: 2,
                layer: 3,
                pressed: true,
            }]
        );
    }

    #[test]
    fn preserves_report_id_prefixed_layer_events_while_waiting_for_vial() {
        let mut layer_event = [0; MAX_INPUT_MESSAGE_LENGTH];
        layer_event[1..8].copy_from_slice(&[b'K', b'M', b'O', 1, 2, 3, 0]);
        let mut layer_events = Vec::new();

        assert_eq!(
            classify_vial_response(layer_event, MAX_INPUT_MESSAGE_LENGTH, &mut layer_events)
                .unwrap(),
            None
        );
        assert_eq!(
            layer_events,
            vec![RawLayerEvent {
                keyboard_id: 2,
                layer: 3,
                pressed: false,
            }]
        );
    }

    #[test]
    fn accepts_complete_non_layer_reports_as_vial_responses() {
        let mut response = [0; MAX_INPUT_MESSAGE_LENGTH];
        response[..3].copy_from_slice(&[CMD_VIA_GET_PROTOCOL_VERSION, 0, 9]);
        let mut layer_events = Vec::new();
        let mut expected = [0; MESSAGE_LENGTH];
        expected[..3].copy_from_slice(&[CMD_VIA_GET_PROTOCOL_VERSION, 0, 9]);

        assert_eq!(
            classify_vial_response(response, MESSAGE_LENGTH, &mut layer_events).unwrap(),
            Some(expected)
        );
        assert!(layer_events.is_empty());
    }

    #[test]
    fn accepts_report_id_prefixed_vial_responses() {
        let mut response = [0; MAX_INPUT_MESSAGE_LENGTH];
        response[1..4].copy_from_slice(&[CMD_VIA_GET_PROTOCOL_VERSION, 0, 9]);
        let mut expected = [0; MESSAGE_LENGTH];
        expected[..3].copy_from_slice(&[CMD_VIA_GET_PROTOCOL_VERSION, 0, 9]);
        let mut layer_events = Vec::new();

        assert_eq!(
            classify_vial_response(response, MAX_INPUT_MESSAGE_LENGTH, &mut layer_events).unwrap(),
            Some(expected)
        );
        assert!(layer_events.is_empty());
    }

    #[test]
    fn rejects_incomplete_reports_while_waiting_for_vial() {
        let error = classify_vial_response(
            [0; MAX_INPUT_MESSAGE_LENGTH],
            MESSAGE_LENGTH - 1,
            &mut Vec::new(),
        )
        .unwrap_err();

        assert!(error.to_string().contains("Incomplete Vial response"));
    }
}

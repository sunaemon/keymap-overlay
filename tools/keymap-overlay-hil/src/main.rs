#[cfg(any(target_os = "linux", target_os = "macos"))]
mod platform {
    use anyhow::{Context, Result, bail};
    use clap::{Parser, Subcommand, ValueEnum};
    use hidapi::{HidApi, HidDevice};
    use keymap_core::{
        HilEncoderDirection, HilLayerState, RAW_HID_REPORT_SIZE, carries_report_magic,
        encode_hil_encoder_command, encode_hil_layer_command, encode_hil_probe_command,
    };
    use keymap_overlay_generator::types::KeymapOverlayMetadata;
    use keymap_overlay_generator::vial::{self, USAGE_ID, USAGE_PAGE};
    use serde_json::Value;
    use std::thread;
    use std::time::Duration;

    const VIA_GET_KEYCODE: u8 = 0x04;
    const VIA_SET_KEYCODE: u8 = 0x05;
    const VIA_RESET_KEYMAP: u8 = 0x06;
    const VIAL_PREFIX: u8 = 0xFE;
    const VIAL_GET_ENCODER: u8 = 0x03;
    const VIAL_SET_ENCODER: u8 = 0x04;
    const RESPONSE_TIMEOUT_MS: i32 = 1_000;
    const HIL_DISPATCH_DELAY: Duration = Duration::from_millis(50);

    #[derive(Debug, Parser)]
    #[command(
        name = "keymap-overlay-hil",
        about = "Drives the keymap-overlay hardware-in-the-loop protocol"
    )]
    struct Arguments {
        #[command(subcommand)]
        command: Command,
    }

    #[derive(Debug, Subcommand)]
    enum Command {
        /// Lists connected self-describing keyboards.
        Devices,
        /// Proves that the selected keyboard has the HIL firmware protocol.
        Probe {
            #[arg(long)]
            keyboard_id: u8,
        },
        /// Asks firmware to emit one overlay-only layer report.
        Layer {
            #[arg(long)]
            keyboard_id: u8,
            #[arg(long)]
            layer: u8,
            #[arg(long)]
            state: State,
        },
        /// Queues one synthetic rotation through QMK's encoder path.
        Rotate {
            #[arg(long)]
            keyboard_id: u8,
            #[arg(long)]
            index: u8,
            #[arg(long)]
            direction: Direction,
        },
        /// Reads one live Vial keycode.
        GetKeycode {
            #[arg(long)]
            keyboard_id: u8,
            #[arg(long)]
            layer: u8,
            #[arg(long)]
            row: u8,
            #[arg(long)]
            column: u8,
        },
        /// Writes one live Vial keycode.
        SetKeycode {
            #[arg(long)]
            keyboard_id: u8,
            #[arg(long)]
            layer: u8,
            #[arg(long)]
            row: u8,
            #[arg(long)]
            column: u8,
            #[arg(long, value_parser = parse_keycode)]
            keycode: u16,
        },
        /// Reads one live Vial encoder binding.
        GetEncoder {
            #[arg(long)]
            keyboard_id: u8,
            #[arg(long)]
            layer: u8,
            #[arg(long)]
            index: u8,
            #[arg(long)]
            direction: Direction,
        },
        /// Writes one live Vial encoder binding.
        SetEncoder {
            #[arg(long)]
            keyboard_id: u8,
            #[arg(long)]
            layer: u8,
            #[arg(long)]
            index: u8,
            #[arg(long)]
            direction: Direction,
            #[arg(long, value_parser = parse_keycode)]
            keycode: u16,
        },
        /// Finds a displayed key inherited from layer zero.
        FindTransparent {
            #[arg(long)]
            keyboard_id: u8,
            #[arg(long)]
            layer: u8,
        },
        /// Resets live Vial EEPROM to the compiled keymap defaults.
        ResetKeymap {
            #[arg(long)]
            keyboard_id: u8,
        },
    }

    #[derive(Clone, Copy, Debug, ValueEnum)]
    enum State {
        Press,
        Release,
    }

    #[derive(Clone, Copy, Debug, ValueEnum)]
    enum Direction {
        Ccw,
        Cw,
    }

    struct ConnectedKeyboard {
        keyboard_id: u8,
        vendor_id: u16,
        product_id: u16,
        device: HidDevice,
        definition: Value,
    }

    pub fn main() -> Result<()> {
        match Arguments::parse().command {
            Command::Devices => list_devices(),
            Command::Probe { keyboard_id } => probe(keyboard_id),
            Command::Layer {
                keyboard_id,
                layer,
                state,
            } => send_layer_event(keyboard_id, layer, state),
            Command::Rotate {
                keyboard_id,
                index,
                direction,
            } => rotate_encoder(keyboard_id, index, direction),
            Command::GetKeycode {
                keyboard_id,
                layer,
                row,
                column,
            } => get_keycode(keyboard_id, layer, row, column),
            Command::SetKeycode {
                keyboard_id,
                layer,
                row,
                column,
                keycode,
            } => set_keycode(keyboard_id, layer, row, column, keycode),
            Command::GetEncoder {
                keyboard_id,
                layer,
                index,
                direction,
            } => get_encoder(keyboard_id, layer, index, direction),
            Command::SetEncoder {
                keyboard_id,
                layer,
                index,
                direction,
                keycode,
            } => set_encoder(keyboard_id, layer, index, direction, keycode),
            Command::FindTransparent { keyboard_id, layer } => find_transparent(keyboard_id, layer),
            Command::ResetKeymap { keyboard_id } => reset_keymap(keyboard_id),
        }
    }
    fn list_devices() -> Result<()> {
        for keyboard in connected_keyboards()? {
            println!(
                "keyboard_id={} usb={:04x}:{:04x}",
                keyboard.keyboard_id, keyboard.vendor_id, keyboard.product_id
            );
        }
        Ok(())
    }

    fn send_layer_event(keyboard_id: u8, layer: u8, state: State) -> Result<()> {
        if layer == 0 {
            bail!("Layer zero is not a momentary overlay layer");
        }
        let keyboard = open_keyboard(keyboard_id)?;
        let state = match state {
            State::Press => HilLayerState::Pressed,
            State::Release => HilLayerState::Released,
        };
        let command = encode_hil_layer_command(layer, state);
        send_hil_command(&keyboard, &command, "layer")?;
        thread::sleep(HIL_DISPATCH_DELAY);
        Ok(())
    }

    fn rotate_encoder(keyboard_id: u8, index: u8, direction: Direction) -> Result<()> {
        let keyboard = open_keyboard(keyboard_id)?;
        validate_encoder_index(&keyboard, index)?;
        let direction = match direction {
            Direction::Ccw => HilEncoderDirection::CounterClockwise,
            Direction::Cw => HilEncoderDirection::Clockwise,
        };
        let command = encode_hil_encoder_command(index, direction);
        send_hil_command(&keyboard, &command, "encoder rotation")?;
        thread::sleep(HIL_DISPATCH_DELAY);
        Ok(())
    }

    fn probe(keyboard_id: u8) -> Result<()> {
        let keyboard = open_keyboard(keyboard_id)?;
        let command = encode_hil_probe_command();
        let response = send_recv(&keyboard.device, &command)?;
        if response[8] != 0 {
            bail!("Keyboard {keyboard_id} rejected the HIL probe");
        }
        println!("keyboard_id={keyboard_id} hil_version={}", response[5]);
        Ok(())
    }

    fn get_keycode(keyboard_id: u8, layer: u8, row: u8, column: u8) -> Result<()> {
        let keyboard = open_keyboard(keyboard_id)?;
        let response = send_recv(&keyboard.device, &[VIA_GET_KEYCODE, layer, row, column])?;
        let keycode = u16::from_be_bytes([response[4], response[5]]);
        println!("0x{keycode:04X}");
        Ok(())
    }

    fn set_keycode(keyboard_id: u8, layer: u8, row: u8, column: u8, keycode: u16) -> Result<()> {
        let keyboard = open_keyboard(keyboard_id)?;
        let [high, low] = keycode.to_be_bytes();
        send_recv(
            &keyboard.device,
            &[VIA_SET_KEYCODE, layer, row, column, high, low],
        )?;
        println!("0x{keycode:04X}");
        Ok(())
    }

    fn get_encoder(keyboard_id: u8, layer: u8, index: u8, direction: Direction) -> Result<()> {
        let keyboard = open_keyboard(keyboard_id)?;
        validate_encoder_index(&keyboard, index)?;
        let response = send_recv(
            &keyboard.device,
            &[VIAL_PREFIX, VIAL_GET_ENCODER, layer, index],
        )?;
        let offset = match direction {
            Direction::Ccw => 0,
            Direction::Cw => 2,
        };
        let keycode = u16::from_be_bytes([response[offset], response[offset + 1]]);
        println!("0x{keycode:04X}");
        Ok(())
    }

    fn set_encoder(
        keyboard_id: u8,
        layer: u8,
        index: u8,
        direction: Direction,
        keycode: u16,
    ) -> Result<()> {
        let keyboard = open_keyboard(keyboard_id)?;
        validate_encoder_index(&keyboard, index)?;
        let clockwise = match direction {
            Direction::Ccw => 0,
            Direction::Cw => 1,
        };
        let [high, low] = keycode.to_be_bytes();
        send_recv(
            &keyboard.device,
            &[
                VIAL_PREFIX,
                VIAL_SET_ENCODER,
                layer,
                index,
                clockwise,
                high,
                low,
            ],
        )?;
        println!("0x{keycode:04X}");
        Ok(())
    }

    fn find_transparent(keyboard_id: u8, layer: u8) -> Result<()> {
        let keyboard = open_keyboard(keyboard_id)?;
        let metadata: KeymapOverlayMetadata = serde_json::from_value(
            keyboard
                .definition
                .get("keymapOverlay")
                .cloned()
                .context("Device definition has no keymapOverlay metadata")?,
        )
        .context("Device keymapOverlay metadata is invalid")?;
        let model = vial::read_device_model_with_definition(
            &keyboard.device,
            keyboard.definition,
            metadata.keyboard.encoder_count(),
        )?;
        if layer == 0 || layer >= model.layer_count {
            bail!(
                "Layer {layer} is outside the device's momentary layer range 1..{}",
                model.layer_count - 1
            );
        }
        let matrix_size = usize::from(model.matrix_rows) * usize::from(model.matrix_cols);
        let layer_offset = usize::from(layer) * matrix_size;
        for row in 0..model.matrix_rows {
            for column in 0..model.matrix_cols {
                let matrix_offset =
                    usize::from(row) * usize::from(model.matrix_cols) + usize::from(column);
                if model.keycodes[layer_offset + matrix_offset] == 0x0001 {
                    println!(
                        "row={row} column={column} original=0x{:04X}",
                        model.keycodes[matrix_offset]
                    );
                    return Ok(());
                }
            }
        }
        bail!("Layer {layer} has no transparent key inherited from layer zero")
    }

    fn reset_keymap(keyboard_id: u8) -> Result<()> {
        let keyboard = open_keyboard(keyboard_id)?;
        send_recv(&keyboard.device, &[VIA_RESET_KEYMAP])?;
        println!("keyboard_id={keyboard_id} reset=compiled-defaults");
        Ok(())
    }

    fn send_hil_command(
        keyboard: &ConnectedKeyboard,
        command: &[u8; RAW_HID_REPORT_SIZE],
        description: &str,
    ) -> Result<()> {
        let response = send_recv(&keyboard.device, command)
            .with_context(|| format!("Failed to send the HIL {description} command"))?;
        if response[8] != 0 {
            bail!(
                "Keyboard {} rejected the HIL {description} command",
                keyboard.keyboard_id
            );
        }
        Ok(())
    }

    fn validate_encoder_index(keyboard: &ConnectedKeyboard, index: u8) -> Result<()> {
        let metadata: KeymapOverlayMetadata = serde_json::from_value(
            keyboard
                .definition
                .get("keymapOverlay")
                .cloned()
                .context("Device definition has no keymapOverlay metadata")?,
        )
        .context("Device keymapOverlay metadata is invalid")?;
        let encoder_count = metadata.keyboard.encoder_count();
        if usize::from(index) >= encoder_count {
            bail!(
                "Encoder index {index} is outside keyboard {}'s range 0..{}",
                keyboard.keyboard_id,
                encoder_count.saturating_sub(1)
            );
        }
        Ok(())
    }

    fn connected_keyboards() -> Result<Vec<ConnectedKeyboard>> {
        let api = HidApi::new().context("Failed to initialize HID API")?;
        let mut keyboards = Vec::new();
        for info in api
            .device_list()
            .filter(|info| info.usage_page() == USAGE_PAGE && info.usage() == USAGE_ID)
        {
            let device = api
                .open_path(info.path())
                .with_context(|| format!("Failed to open Raw HID device {:?}", info.path()))?;
            let definition = vial::read_device_definition(&device)
                .with_context(|| format!("Failed to read Vial device {:?}", info.path()))?;
            let Some(keyboard_id) = definition
                .pointer("/keymapOverlay/keyboardId")
                .and_then(serde_json::Value::as_u64)
                .and_then(|value| u8::try_from(value).ok())
            else {
                continue;
            };
            keyboards.push(ConnectedKeyboard {
                keyboard_id,
                vendor_id: info.vendor_id(),
                product_id: info.product_id(),
                device,
                definition,
            });
        }
        Ok(keyboards)
    }

    fn open_keyboard(keyboard_id: u8) -> Result<ConnectedKeyboard> {
        let mut matches = connected_keyboards()?
            .into_iter()
            .filter(|keyboard| keyboard.keyboard_id == keyboard_id);
        let Some(keyboard) = matches.next() else {
            bail!("No connected self-describing keyboard uses KEYBOARD_ID {keyboard_id}");
        };
        if matches.next().is_some() {
            bail!("More than one connected keyboard uses KEYBOARD_ID {keyboard_id}");
        }
        Ok(keyboard)
    }

    fn write_report(device: &HidDevice, payload: &[u8; RAW_HID_REPORT_SIZE]) -> Result<()> {
        let mut report = [0_u8; RAW_HID_REPORT_SIZE + 1];
        report[1..].copy_from_slice(payload);
        let written = device.write(&report).context("Raw HID write failed")?;
        if written != report.len() {
            bail!(
                "Incomplete Raw HID write: expected {} bytes, wrote {written}",
                report.len()
            );
        }
        Ok(())
    }

    fn send_recv(device: &HidDevice, request: &[u8]) -> Result<[u8; RAW_HID_REPORT_SIZE]> {
        if request.len() > RAW_HID_REPORT_SIZE {
            bail!("Vial request exceeds the Raw HID report size");
        }
        let mut payload = [0_u8; RAW_HID_REPORT_SIZE];
        payload[..request.len()].copy_from_slice(request);
        write_report(device, &payload)?;

        let mut response = [0_u8; RAW_HID_REPORT_SIZE];
        loop {
            let count = device
                .read_timeout(&mut response, RESPONSE_TIMEOUT_MS)
                .context("Failed while waiting for the Vial response")?;
            if count == 0 {
                bail!("Timed out waiting for the Vial response");
            }
            let encoder_read = request.starts_with(&[VIAL_PREFIX, VIAL_GET_ENCODER])
                && !carries_report_magic(&response);
            if count == RAW_HID_REPORT_SIZE && (response[0] == request[0] || encoder_read) {
                return Ok(response);
            }
        }
    }

    fn parse_keycode(value: &str) -> Result<u16, String> {
        let digits = value
            .strip_prefix("0x")
            .or_else(|| value.strip_prefix("0X"))
            .unwrap_or(value);
        u16::from_str_radix(digits, 16)
            .map_err(|error| format!("expected a 16-bit hexadecimal keycode: {error}"))
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn parses_prefixed_and_unprefixed_keycodes_as_hexadecimal() {
            assert_eq!(parse_keycode("0x0068"), Ok(0x0068));
            assert_eq!(parse_keycode("7C00"), Ok(0x7C00));
        }

        #[test]
        fn rejects_a_keycode_outside_sixteen_bits() {
            assert!(parse_keycode("0x10000").is_err());
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn main() -> anyhow::Result<()> {
    platform::main()
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn main() {
    eprintln!("keymap-overlay-hil is currently available only on Linux and macOS");
    std::process::exit(1);
}

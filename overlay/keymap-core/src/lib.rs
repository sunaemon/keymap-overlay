//! The Raw HID protocol and active-layer state machine.
//!
//! The firmware sends a 32-byte report on every momentary layer press and
//! release. The reducer folds those reports and device disconnections into the
//! final active-layer change; see `docs/design.md` for the wire format.

pub const RAW_HID_REPORT_MAGIC: [u8; 3] = *b"KMO";
pub const RAW_HID_REPORT_VERSION: u8 = 1;
pub const RAW_HID_REPORT_SIZE: usize = 32;
/// VIA custom command reserved for hardware-in-the-loop layer-event tests.
pub const HIL_COMMAND_ID: u8 = 0xFC;
/// Distinguishes the HIL request from another keyboard-specific VIA command.
pub const HIL_COMMAND_MAGIC: [u8; 4] = *b"KMOH";
/// Version of the host-to-firmware HIL request.
pub const HIL_COMMAND_VERSION: u8 = 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HilLayerState {
    Pressed,
    Released,
}

/// Builds a side-effect-free request that proves the HIL firmware is present.
pub fn encode_hil_probe_command() -> [u8; RAW_HID_REPORT_SIZE] {
    let mut report = [0_u8; RAW_HID_REPORT_SIZE];
    report[0] = HIL_COMMAND_ID;
    report[1..5].copy_from_slice(&HIL_COMMAND_MAGIC);
    report[5] = HIL_COMMAND_VERSION;
    report
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RawLayerEvent {
    pub keyboard_id: u8,
    pub layer: u8,
    pub pressed: bool,
}

/// Builds the fixed-size VIA request for one deterministic layer event.
pub fn encode_hil_layer_command(layer: u8, state: HilLayerState) -> [u8; RAW_HID_REPORT_SIZE] {
    let mut report = [0_u8; RAW_HID_REPORT_SIZE];
    report[0] = HIL_COMMAND_ID;
    report[1..5].copy_from_slice(&HIL_COMMAND_MAGIC);
    report[5] = HIL_COMMAND_VERSION;
    report[6] = match state {
        HilLayerState::Pressed => 1,
        HilLayerState::Released => 2,
    };
    report[7] = layer;
    report
}

/// An input to the active-layer state machine.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LayerEvent {
    Report(RawLayerEvent),
    Disconnected { keyboard_id: Option<u8> },
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum ActiveLayerChange {
    #[default]
    Unchanged,
    Changed(Option<ActiveLayerState>),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActiveLayerState {
    pub keyboard_id: u8,
    pub layers: Vec<u8>,
}

/// Held layers and the final state change waiting to be consumed.
///
/// Several inputs can arrive before a UI loop handles them. Intermediate
/// restores and switches are reduced here so consumers observe only the final
/// active-layer state.
#[derive(Default)]
pub struct PendingLayerChange {
    held_keys: Vec<(u8, u8)>,
    pending: ActiveLayerChange,
}

impl PendingLayerChange {
    /// Folds one input into the held-layer state and keeps its latest change.
    pub fn push(&mut self, event: LayerEvent) {
        let change = transition_for_event(&mut self.held_keys, event);
        if change != ActiveLayerChange::Unchanged {
            self.pending = change;
        }
    }

    /// Takes the final state change, leaving no pending change behind.
    pub fn take(&mut self) -> ActiveLayerChange {
        std::mem::take(&mut self.pending)
    }
}

/// Updates held momentary layers for one report or device disconnection.
fn transition_for_event(held_keys: &mut Vec<(u8, u8)>, event: LayerEvent) -> ActiveLayerChange {
    match event {
        LayerEvent::Report(event) => transition_for(held_keys, event),
        LayerEvent::Disconnected { keyboard_id } => {
            transition_for_disconnect(held_keys, keyboard_id)
        }
    }
}

/// Updates held momentary layers and reports whether the active layer changed.
fn transition_for(held_keys: &mut Vec<(u8, u8)>, event: RawLayerEvent) -> ActiveLayerChange {
    let previous = active_layer_state(held_keys);
    let key = (event.keyboard_id, event.layer);
    if event.pressed {
        held_keys.push(key);
    } else {
        let Some(index) = held_keys.iter().position(|held_key| *held_key == key) else {
            return ActiveLayerChange::Unchanged;
        };
        held_keys.remove(index);
    }
    change_between(previous, active_layer_state(held_keys))
}

/// Removes held layers belonging to a disconnected keyboard.
fn transition_for_disconnect(
    held_keys: &mut Vec<(u8, u8)>,
    keyboard_id: Option<u8>,
) -> ActiveLayerChange {
    let Some(keyboard_id) = keyboard_id else {
        return ActiveLayerChange::Unchanged;
    };
    let previous = active_layer_state(held_keys);
    held_keys.retain(|(held_keyboard_id, _)| *held_keyboard_id != keyboard_id);
    change_between(previous, active_layer_state(held_keys))
}

fn active_layer_state(held_keys: &[(u8, u8)]) -> Option<ActiveLayerState> {
    let keyboard_id = held_keys.last()?.0;
    let mut layers = held_keys
        .iter()
        .filter_map(|(held_keyboard_id, layer)| {
            (*held_keyboard_id == keyboard_id).then_some(*layer)
        })
        .collect::<Vec<_>>();
    layers.sort_unstable();
    layers.dedup();
    Some(ActiveLayerState {
        keyboard_id,
        layers,
    })
}

fn change_between(
    previous: Option<ActiveLayerState>,
    current: Option<ActiveLayerState>,
) -> ActiveLayerChange {
    if previous == current {
        ActiveLayerChange::Unchanged
    } else {
        ActiveLayerChange::Changed(current)
    }
}

/// Drops the zero report ID some hosts prepend, leaving the firmware's frame.
///
/// Both readers below go through this so the rule is stated once: change the
/// framing here and the magic check and the parser stay in agreement.
fn strip_report_id(report: &[u8]) -> &[u8] {
    match report {
        [0, frame @ ..] => frame,
        _ => report,
    }
}

/// Reports whether a frame claims to be ours, with or without a report ID.
///
/// Callers use this to tell unrelated traffic sharing the interface from a
/// frame the firmware meant for us but that this version cannot parse.
pub fn carries_report_magic(report: &[u8]) -> bool {
    strip_report_id(report).starts_with(&RAW_HID_REPORT_MAGIC)
}

pub fn parse_raw_layer_event(report: &[u8]) -> Option<RawLayerEvent> {
    // The firmware always sends a full RAW_HID_REPORT_SIZE report. A shorter
    // frame is either truncated or belongs to another protocol sharing the
    // interface, such as VIAL.
    let frame = strip_report_id(report);
    if frame.len() < RAW_HID_REPORT_SIZE {
        return None;
    }
    let payload = frame.strip_prefix(&RAW_HID_REPORT_MAGIC)?;
    let [version, keyboard_id, layer, pressed, ..] = payload else {
        return None;
    };
    if *version != RAW_HID_REPORT_VERSION {
        return None;
    }
    let pressed = match *pressed {
        0 => false,
        1 => true,
        _ => return None,
    };

    Some(RawLayerEvent {
        keyboard_id: *keyboard_id,
        layer: *layer,
        pressed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds the zero-padded report the firmware actually sends.
    fn report(version: u8, keyboard_id: u8, layer: u8, pressed: u8) -> [u8; RAW_HID_REPORT_SIZE] {
        let mut report = [0_u8; RAW_HID_REPORT_SIZE];
        report[..3].copy_from_slice(&RAW_HID_REPORT_MAGIC);
        report[3] = version;
        report[4] = keyboard_id;
        report[5] = layer;
        report[6] = pressed;
        report
    }

    /// Prepends the zero report ID that some hosts add.
    fn with_report_id(report: &[u8]) -> Vec<u8> {
        let mut prefixed = vec![0_u8];
        prefixed.extend_from_slice(report);
        prefixed
    }

    #[test]
    fn encodes_a_narrow_hil_layer_request() {
        let press = encode_hil_layer_command(3, HilLayerState::Pressed);
        assert_eq!(press[0], HIL_COMMAND_ID);
        assert_eq!(&press[1..5], &HIL_COMMAND_MAGIC);
        assert_eq!(press[5], HIL_COMMAND_VERSION);
        assert_eq!(press[6], 1);
        assert_eq!(press[7], 3);
        assert!(press[8..].iter().all(|byte| *byte == 0));

        let release = encode_hil_layer_command(3, HilLayerState::Released);
        assert_eq!(release[6], 2);
    }

    #[test]
    fn encodes_a_side_effect_free_hil_probe() {
        let probe = encode_hil_probe_command();
        assert_eq!(probe[0], HIL_COMMAND_ID);
        assert_eq!(&probe[1..5], &HIL_COMMAND_MAGIC);
        assert_eq!(probe[5], HIL_COMMAND_VERSION);
        assert!(probe[6..].iter().all(|byte| *byte == 0));
    }

    fn event(keyboard_id: u8, layer: u8, pressed: bool) -> RawLayerEvent {
        RawLayerEvent {
            keyboard_id,
            layer,
            pressed,
        }
    }

    fn state(keyboard_id: u8, layers: &[u8]) -> Option<ActiveLayerState> {
        Some(ActiveLayerState {
            keyboard_id,
            layers: layers.to_vec(),
        })
    }

    #[test]
    fn pressing_a_layer_makes_it_active() {
        assert_eq!(
            transition_for(&mut vec![], event(1, 2, true)),
            ActiveLayerChange::Changed(state(1, &[2]))
        );
    }

    #[test]
    fn releasing_the_active_layer_restores_the_previous_layer() {
        let mut held_keys = vec![(1, 2), (1, 3)];

        assert_eq!(
            transition_for(&mut held_keys, event(1, 3, false)),
            ActiveLayerChange::Changed(state(1, &[2]))
        );
        assert_eq!(held_keys, vec![(1, 2)]);
    }

    #[test]
    fn pressing_a_lower_layer_keeps_layers_in_qmk_precedence_order() {
        let mut held_keys = vec![(1, 3)];

        assert_eq!(
            transition_for(&mut held_keys, event(1, 1, true)),
            ActiveLayerChange::Changed(state(1, &[1, 3]))
        );
    }

    #[test]
    fn releasing_a_lower_layer_updates_transparency_state() {
        let mut held_keys = vec![(1, 2), (1, 3)];

        assert_eq!(
            transition_for(&mut held_keys, event(1, 2, false)),
            ActiveLayerChange::Changed(state(1, &[3]))
        );
        assert_eq!(held_keys, vec![(1, 3)]);
    }

    #[test]
    fn duplicate_layer_keys_stay_active_until_both_are_released() {
        let mut held_keys = vec![];

        assert_eq!(
            transition_for(&mut held_keys, event(1, 2, true)),
            ActiveLayerChange::Changed(state(1, &[2]))
        );
        assert_eq!(
            transition_for(&mut held_keys, event(1, 2, true)),
            ActiveLayerChange::Unchanged
        );
        assert_eq!(held_keys, vec![(1, 2), (1, 2)]);

        assert_eq!(
            transition_for(&mut held_keys, event(1, 2, false)),
            ActiveLayerChange::Unchanged
        );
        assert_eq!(held_keys, vec![(1, 2)]);
        assert_eq!(
            transition_for(&mut held_keys, event(1, 2, false)),
            ActiveLayerChange::Changed(None)
        );
    }

    #[test]
    fn disconnecting_the_active_keyboard_restores_another_keyboard() {
        let mut held_keys = vec![(2, 3), (1, 2)];

        assert_eq!(
            transition_for_disconnect(&mut held_keys, Some(1)),
            ActiveLayerChange::Changed(state(2, &[3]))
        );
        assert_eq!(held_keys, vec![(2, 3)]);
    }

    #[test]
    fn disconnecting_the_only_active_keyboard_hides_the_overlay() {
        let mut held_keys = vec![(1, 2)];

        assert_eq!(
            transition_for_disconnect(&mut held_keys, Some(1)),
            ActiveLayerChange::Changed(None)
        );
        assert!(held_keys.is_empty());
    }

    #[test]
    fn disconnecting_an_unknown_keyboard_changes_nothing() {
        let mut held_keys = vec![(1, 2)];

        assert_eq!(
            transition_for_disconnect(&mut held_keys, Some(2)),
            ActiveLayerChange::Unchanged
        );
        assert_eq!(held_keys, vec![(1, 2)]);
    }

    #[test]
    fn queued_events_only_expose_their_final_state() {
        let mut pending = PendingLayerChange::default();
        pending.push(LayerEvent::Report(event(1, 2, true)));
        assert_eq!(pending.take(), ActiveLayerChange::Changed(state(1, &[2])));

        for event in [event(1, 3, true), event(1, 3, false), event(1, 2, false)] {
            pending.push(LayerEvent::Report(event));
        }

        assert_eq!(pending.take(), ActiveLayerChange::Changed(None));
    }

    /// Taking a change must not hand the same update out a second time.
    #[test]
    fn nothing_is_pending_once_a_change_has_been_taken() {
        let mut pending = PendingLayerChange::default();
        pending.push(LayerEvent::Report(event(1, 2, true)));

        assert_eq!(pending.take(), ActiveLayerChange::Changed(state(1, &[2])));
        assert_eq!(pending.take(), ActiveLayerChange::Unchanged);
    }

    /// An unchanged input must not clear a state change waiting to be consumed.
    #[test]
    fn an_unchanged_event_leaves_an_earlier_change_pending() {
        let mut pending = PendingLayerChange::default();
        pending.push(LayerEvent::Report(event(1, 2, true)));
        pending.push(LayerEvent::Disconnected {
            keyboard_id: Some(9),
        });

        assert_eq!(pending.take(), ActiveLayerChange::Changed(state(1, &[2])));
    }

    #[test]
    fn parses_raw_hid_layer_events_with_or_without_a_report_id() {
        let event = RawLayerEvent {
            keyboard_id: 2,
            layer: 3,
            pressed: true,
        };
        let report = report(RAW_HID_REPORT_VERSION, 2, 3, 1);

        assert_eq!(parse_raw_layer_event(&report), Some(event));
        assert_eq!(parse_raw_layer_event(&with_report_id(&report)), Some(event));
    }

    #[test]
    fn reads_the_release_flag() {
        assert_eq!(
            parse_raw_layer_event(&report(RAW_HID_REPORT_VERSION, 1, 2, 0)),
            Some(RawLayerEvent {
                keyboard_id: 1,
                layer: 2,
                pressed: false,
            })
        );
    }

    #[test]
    fn rejects_an_invalid_pressed_flag() {
        assert_eq!(
            parse_raw_layer_event(&report(RAW_HID_REPORT_VERSION, 1, 2, 2)),
            None
        );
    }

    #[test]
    fn rejects_reports_that_are_not_the_kmo_protocol() {
        // VIAL shares the Raw HID interface, so unrelated traffic must be ignored.
        assert_eq!(parse_raw_layer_event(&[]), None);
        assert_eq!(parse_raw_layer_event(b"VIAL"), None);
        assert_eq!(
            parse_raw_layer_event(&report(RAW_HID_REPORT_VERSION + 1, 1, 2, 1)),
            None
        );
    }

    #[test]
    fn recognizes_the_magic_with_or_without_a_report_id() {
        let report = report(RAW_HID_REPORT_VERSION, 1, 2, 1);

        assert!(carries_report_magic(&report));
        assert!(carries_report_magic(&with_report_id(&report)));
        // Short frames still count: the magic is what identifies the sender.
        assert!(carries_report_magic(b"KMO"));
        assert!(carries_report_magic(&[0, b'K', b'M', b'O']));
    }

    #[test]
    fn does_not_claim_frames_belonging_to_other_protocols() {
        assert!(!carries_report_magic(&[]));
        assert!(!carries_report_magic(b"VIAL"));
        assert!(!carries_report_magic(b"KM"));
        // Only one leading zero is a report ID; two is someone else's frame.
        assert!(!carries_report_magic(&[0, 0, b'K', b'M', b'O']));
    }

    /// A version bump is the case the magic check exists to make visible.
    #[test]
    fn a_future_version_carries_the_magic_but_does_not_parse() {
        let report = report(RAW_HID_REPORT_VERSION + 1, 1, 2, 1);

        assert!(carries_report_magic(&report));
        assert_eq!(parse_raw_layer_event(&report), None);
    }

    #[test]
    fn rejects_reports_shorter_than_a_full_frame() {
        let report = report(RAW_HID_REPORT_VERSION, 2, 3, 1);

        assert_eq!(parse_raw_layer_event(b"KMO"), None);
        assert_eq!(parse_raw_layer_event(&report[..7]), None);
        assert_eq!(
            parse_raw_layer_event(&report[..RAW_HID_REPORT_SIZE - 1]),
            None
        );
        // A report ID makes the frame one byte longer, so 32 bytes is short.
        assert_eq!(
            parse_raw_layer_event(&with_report_id(&report)[..RAW_HID_REPORT_SIZE]),
            None
        );
    }
}

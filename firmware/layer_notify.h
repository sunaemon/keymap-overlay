// Copyright 2026 sunaemon
// SPDX-License-Identifier: GPL-2.0-or-later

#pragma once

#include "eeconfig.h"
#include "raw_hid.h"

#ifdef ENCODER_ENABLE
#include "encoder.h"
#endif

#if defined(ENCODER_ENABLE) && defined(ENCODER_MAP_ENABLE)
#include "action_layer.h"
#include "keyboard.h"
#include "keymap_common.h"
#endif

#ifndef KEYMAP_EEPROM_EPOCH
#define KEYMAP_EEPROM_EPOCH 0
#endif

#ifndef KEYBOARD_ID
#error                                                                         \
    "KEYBOARD_ID is not defined; add OPT_DEFS += -DKEYBOARD_ID=<n> to rules.mk"
#endif

// The overlay reads the keyboard ID out of a single report byte and uses it to
// pick <KEYBOARD_ID>_L<layer>.png, so it has to fit in a uint8_t.
_Static_assert(KEYBOARD_ID >= 0 && KEYBOARD_ID <= 255,
               "KEYBOARD_ID must be an integer between 0 and 255");

#define KEYMAP_OVERLAY_REPORT_MAGIC_0 'K'
#define KEYMAP_OVERLAY_REPORT_MAGIC_1 'M'
#define KEYMAP_OVERLAY_REPORT_MAGIC_2 'O'
#define KEYMAP_OVERLAY_REPORT_VERSION 1
#define KEYMAP_OVERLAY_REPORT_SIZE 32

#ifdef KEYMAP_OVERLAY_HIL_ENABLE
#define KEYMAP_OVERLAY_HIL_COMMAND_ID 0xFC
#define KEYMAP_OVERLAY_HIL_COMMAND_MAGIC_0 'K'
#define KEYMAP_OVERLAY_HIL_COMMAND_MAGIC_1 'M'
#define KEYMAP_OVERLAY_HIL_COMMAND_MAGIC_2 'O'
#define KEYMAP_OVERLAY_HIL_COMMAND_MAGIC_3 'H'
#define KEYMAP_OVERLAY_HIL_COMMAND_VERSION 1
#define KEYMAP_OVERLAY_HIL_PROBE 0
#define KEYMAP_OVERLAY_HIL_PRESS 1
#define KEYMAP_OVERLAY_HIL_RELEASE 2
#define KEYMAP_OVERLAY_HIL_ENCODER_CCW 3
#define KEYMAP_OVERLAY_HIL_ENCODER_CW 4

static bool keymap_overlay_hil_event_pending;
static uint8_t keymap_overlay_hil_pending_action;
static uint8_t keymap_overlay_hil_pending_value;
#endif

static inline void keymap_overlay_send_layer_event(uint8_t layer,
                                                   bool pressed) {
  uint8_t report[KEYMAP_OVERLAY_REPORT_SIZE] = {0};
  report[0] = KEYMAP_OVERLAY_REPORT_MAGIC_0;
  report[1] = KEYMAP_OVERLAY_REPORT_MAGIC_1;
  report[2] = KEYMAP_OVERLAY_REPORT_MAGIC_2;
  report[3] = KEYMAP_OVERLAY_REPORT_VERSION;
  report[4] = KEYBOARD_ID;
  report[5] = layer;
  report[6] = pressed;
  raw_hid_send(report, sizeof(report));
}

// Notifies the overlay when a momentary layer key (MO) is pressed or released.
//
// Call this at the top of process_record_user. It only reports the event; QMK
// still performs the layer switch itself, so the caller must go on to return
// true for the key to work. Non-momentary layer switches (TO, TG, LT, LM) are
// deliberately ignored: the overlay hides on the matching release, and a layer
// that stays on would leave it on screen indefinitely.
static inline void keymap_overlay_notify_momentary_layer(uint16_t keycode,
                                                         keyrecord_t *record) {
  if (keycode < MO(1) || keycode > MO(DYNAMIC_KEYMAP_LAYER_COUNT - 1)) {
    return;
  }
  keymap_overlay_send_layer_event(QK_MOMENTARY_GET_LAYER(keycode),
                                  record->event.pressed);
}

#ifdef KEYMAP_OVERLAY_HIL_ENABLE
// Accepts deterministic overlay-report requests and, on encoder keyboards,
// encoder indices that enter the same bounded queue as physical rotation. It
// cannot choose an arbitrary keycode, change the active QMK layer, or write
// EEPROM. Reject the effective QK_BOOT binding before queueing an encoder
// event, so this command cannot enter the bootloader through the live keymap.
void raw_hid_receive_kb(uint8_t *data, uint8_t length) {
  if (length != KEYMAP_OVERLAY_REPORT_SIZE ||
      data[0] != KEYMAP_OVERLAY_HIL_COMMAND_ID ||
      data[1] != KEYMAP_OVERLAY_HIL_COMMAND_MAGIC_0 ||
      data[2] != KEYMAP_OVERLAY_HIL_COMMAND_MAGIC_1 ||
      data[3] != KEYMAP_OVERLAY_HIL_COMMAND_MAGIC_2 ||
      data[4] != KEYMAP_OVERLAY_HIL_COMMAND_MAGIC_3 ||
      data[5] != KEYMAP_OVERLAY_HIL_COMMAND_VERSION) {
    data[0] = 0xFF;
    return;
  }

  const uint8_t action = data[6];
  const uint8_t value = data[7];
  if (action == KEYMAP_OVERLAY_HIL_PROBE) {
    data[8] = 0;
    return;
  }

  const bool layer_action = action == KEYMAP_OVERLAY_HIL_PRESS ||
                            action == KEYMAP_OVERLAY_HIL_RELEASE;
  if (layer_action && value > 0 && value < DYNAMIC_KEYMAP_LAYER_COUNT) {
    keymap_overlay_hil_pending_action = action;
    keymap_overlay_hil_pending_value = value;
    keymap_overlay_hil_event_pending = true;
    data[8] = 0;
    return;
  }

#if defined(ENCODER_ENABLE) && defined(ENCODER_MAP_ENABLE)
  const bool encoder_action = action == KEYMAP_OVERLAY_HIL_ENCODER_CCW ||
                              action == KEYMAP_OVERLAY_HIL_ENCODER_CW;
  if (encoder_action && value < NUM_ENCODERS) {
    const bool clockwise = action == KEYMAP_OVERLAY_HIL_ENCODER_CW;
    const keypos_t key = {.row = clockwise ? KEYLOC_ENCODER_CW
                                           : KEYLOC_ENCODER_CCW,
                          .col = value};
    const uint8_t layer = layer_switch_get_layer(key);
    if (keymap_key_to_keycode(layer, key) == QK_BOOT) {
      data[8] = 1;
      return;
    }
    keymap_overlay_hil_pending_action = action;
    keymap_overlay_hil_pending_value = value;
    keymap_overlay_hil_event_pending = true;
    data[8] = 0;
    return;
  }
#endif

  data[8] = 1;
}

// VIA sends the command response after raw_hid_receive_kb returns. Defer the
// unsolicited KMO report or encoder event until the next housekeeping pass so
// the response never overlaps another Raw HID or keyboard report.
void housekeeping_task_user(void) {
  if (!keymap_overlay_hil_event_pending) {
    return;
  }
  keymap_overlay_hil_event_pending = false;

  if (keymap_overlay_hil_pending_action == KEYMAP_OVERLAY_HIL_PRESS ||
      keymap_overlay_hil_pending_action == KEYMAP_OVERLAY_HIL_RELEASE) {
    keymap_overlay_send_layer_event(keymap_overlay_hil_pending_value,
                                    keymap_overlay_hil_pending_action ==
                                        KEYMAP_OVERLAY_HIL_PRESS);
    return;
  }

#if defined(ENCODER_ENABLE) && defined(ENCODER_MAP_ENABLE)
  (void)encoder_queue_event(keymap_overlay_hil_pending_value,
                            keymap_overlay_hil_pending_action ==
                                KEYMAP_OVERLAY_HIL_ENCODER_CW);
#endif
}
#endif

// A firmware flash supplies a fresh epoch. On its first boot, reset all QMK
// and Vial EEPROM so the compiled keymaps and encoder bindings become the
// Vial defaults. Later Vial-app edits persist until the next firmware flash.
static inline void keymap_overlay_reset_eeprom_after_flash(void) {
  if (eeconfig_read_user() != KEYMAP_EEPROM_EPOCH) {
    eeconfig_init();
  }
}

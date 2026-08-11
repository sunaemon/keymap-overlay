// Copyright 2026 sunaemon
// SPDX-License-Identifier: GPL-2.0-or-later

#pragma once

#include "raw_hid.h"

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

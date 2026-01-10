/* Copyright 2022 DOIO
 * Copyright 2022 HorrorTroll <https://github.com/HorrorTroll>
 *
 * This program is free software: you can redistribute it and/or modify
 * it under the terms of the GNU General Public License as published by
 * the Free Software Foundation, either version 2 of the License, or
 * (at your option) any later version.
 *
 * This program is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
 * GNU General Public License for more details.
 *
 * You should have received a copy of the GNU General Public License
 * along with this program.  If not, see <http://www.gnu.org/licenses/>.
 */

#include QMK_KEYBOARD_H
#include "raw_hid.h"

// OLED animation
#include "lib/layer_status/layer_status.h"

// make compile QMK_KEYBOARD=doio/kb16/rev2 enum custom_keycodes {};

// clang-format off
const uint16_t PROGMEM keymaps[DYNAMIC_KEYMAP_LAYER_COUNT][MATRIX_ROWS][MATRIX_COLS] = {
    [0] = LAYOUT(
           KC_1,    KC_2,    KC_3,    KC_4,       KC_MPLY,
           KC_5,    KC_6,    KC_7,    KC_8,       TO(1),
           KC_9,    KC_0,   KC_UP,  KC_ENT,       KC_MUTE,
           MO(3), KC_LEFT, KC_DOWN,KC_RIGHT
    ),
    [1] = LAYOUT(
        _______, _______, _______, _______,       _______,
        _______, _______, _______, _______,      TO(2),
        _______, _______, _______, _______,       _______,
        _______, _______, _______, _______
    ),
    [2] = LAYOUT(
        _______, _______, _______, _______,       _______,
        _______, _______, _______, _______,      TO(0),
        _______, _______, _______, _______,       _______,
        _______, _______, _______, _______
    ),
    [3] = LAYOUT(
        RM_SPDU, RM_SPDD, _______, QK_BOOT,      _______,
        RM_SATU, RM_SATD, _______, _______,      _______,
        RM_TOGG, RM_NEXT, RM_HUEU, _______,      _______,
        _______, RM_VALU, RM_HUED, RM_VALD
    ),
};
// clang-format on

#ifdef OLED_ENABLE
bool oled_task_user(void) {
  render_layer_status();

  return true;
}
#endif

#ifdef ENCODER_MAP_ENABLE
const uint16_t PROGMEM
    encoder_map[DYNAMIC_KEYMAP_LAYER_COUNT][NUM_ENCODERS][NUM_DIRECTIONS] = {
        [0] = {ENCODER_CCW_CW(KC_MPRV, KC_MNXT),
               ENCODER_CCW_CW(KC_PGDN, KC_PGUP),
               ENCODER_CCW_CW(KC_VOLD, KC_VOLU)},
        [1] = {ENCODER_CCW_CW(KC_TRNS, KC_TRNS),
               ENCODER_CCW_CW(KC_TRNS, KC_TRNS),
               ENCODER_CCW_CW(KC_TRNS, KC_TRNS)},
        [2] = {ENCODER_CCW_CW(KC_TRNS, KC_TRNS),
               ENCODER_CCW_CW(KC_TRNS, KC_TRNS),
               ENCODER_CCW_CW(KC_TRNS, KC_TRNS)},
        [3] = {ENCODER_CCW_CW(KC_TRNS, KC_TRNS),
               ENCODER_CCW_CW(KC_TRNS, KC_TRNS),
               ENCODER_CCW_CW(KC_TRNS, KC_TRNS)},
};
#endif

#if KEYBOARD_ID < 0 || KEYBOARD_ID > 127
#error "KEYBOARD_ID must be between 0 and 127"
#endif

typedef struct __attribute__((packed)) {
  uint8_t id;
  uint8_t current_layer;
  uint8_t status; // bit 0-6: keyboard_id, bit 7: pressed flag
  uint8_t padding[29];
} layer_notify_report;

_Static_assert(sizeof(layer_notify_report) == 32,
               "layer_notify_report must be 32 bytes");

layer_state_t layer_state_set_user(layer_state_t state) {
  uint8_t layer = get_highest_layer(state);

  layer_notify_report report = {
      .id = 0x24,
      .current_layer = layer,
      .status = (uint8_t)(KEYBOARD_ID & 0x7F) | 0x80,
  };
  raw_hid_send((uint8_t *)&report, sizeof(report));

  return state;
}

// Process custom keycodes
// Return
// - false to skip all further processing of this key
// - true to continue processing this key
bool process_record_user(uint16_t keycode, keyrecord_t *record) { return true; }

/* Copyright 2022 DOIO
 * Copyright 2022 HorrorTroll <https://github.com/HorrorTroll>
 * Copyright 2026 Sunaemon
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
#include "layer_notify.h"

// OLED animation
#include "lib/layer_status/layer_status.h"

enum custom_keycodes {
  DUMMY_KEY = QK_KB_0,
};

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
        QK_BOOT, _______, _______, _______,      _______,
        _______, _______, _______, _______,      _______,
        _______, _______, _______, _______,      _______,
        _______, _______, _______, _______
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

// Process custom keycodes
// Return
// - false to skip all further processing of this key
// - true to continue processing this key
bool process_record_user(uint16_t keycode, keyrecord_t *record) {
  keymap_overlay_notify_momentary_layer(keycode, record);
  return true;
}

/* Copyright 2025 Salicylic_acid3
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

enum custom_keycodes {
  KC_ALPHA = SAFE_RANGE, // α
  KC_BETA,               // β
  KC_GAMMA,              // γ
  KC_DELTA,              // δ
  KC_EPSILON,            // ε
  KC_ZETA,               // ζ
  KC_ETA,                // η
  KC_THETA,              // θ
  KC_IOTA,               // ι
  KC_KAPPA,              // κ
  KC_LAMBDA,             // λ
  KC_MU,                 // μ
  KC_NU,                 // ν
  KC_XI,                 // ξ
  KC_OMICRON,            // ο
  KC_PI,                 // π
  KC_RHO,                // ρ
  KC_SIGMA,              // σ
  KC_TAU,                // τ
  KC_UPSILON,            // υ
  KC_PHI,                // φ
  KC_CHI,                // χ
  KC_PSI,                // ψ
  KC_OMEGA               // ω
};

// clang-format off
const uint16_t PROGMEM keymaps[DYNAMIC_KEYMAP_LAYER_COUNT][MATRIX_ROWS][MATRIX_COLS] = {
    [0] = LAYOUT(
         KC_ESC,    KC_1,    KC_2,    KC_3,    KC_4,    KC_5,        KC_6,    KC_7,    KC_8,    KC_9,    KC_0, KC_MINS,  KC_EQL, KC_BSLS,  KC_GRV,
         KC_TAB,    KC_Q,    KC_W,    KC_E,    KC_R,    KC_T,        KC_Y,    KC_U,    KC_I,    KC_O,    KC_P, KC_LBRC, KC_RBRC, KC_BSPC,
        KC_LCTL,    KC_A,    KC_S,    KC_D,    KC_F,    KC_G,        KC_H,    KC_J,    KC_K,    KC_L, KC_SCLN, KC_QUOT,  KC_ENT,
        KC_LSFT,    KC_Z,    KC_X,    KC_C,    KC_V,    KC_B,        KC_N,    KC_M, KC_COMM,  KC_DOT, KC_SLSH, KC_RSFT,   MO(1),
          MO(2), KC_LALT, KC_LGUI,  KC_SPC,  KC_SPC,               KC_SPC,  KC_SPC,          KC_RGUI, KC_RALT,  KC_APP, KC_RCTL
    ),
    [1] = LAYOUT(
        _______,   KC_F1,   KC_F2,   KC_F3,   KC_F4,   KC_F5,       KC_F6,   KC_F7,   KC_F8,   KC_F9,  KC_F10,  KC_F11,  KC_F12,  KC_INS,  KC_DEL,
        _______, _______, _______, _______, _______, _______,     _______, _______, KC_PSCR, KC_SCRL,KC_PAUSE,   KC_UP, _______, KC_BSPC,
        _______, _______, _______, _______, _______, _______,     _______, _______, KC_HOME, KC_PGUP, KC_LEFT,KC_RIGHT, _______,
        _______, _______, _______, _______, _______, _______,     _______, _______,  KC_END, KC_PGDN, KC_DOWN, _______, _______,
        _______, _______, _______, _______, _______,              _______, _______,          KC_STOP, _______, _______, _______
    ),
    [2] = LAYOUT(
        _______, _______, _______, _______, _______, _______,     _______, _______, _______, _______, _______, _______, _______, _______, _______,
        _______, _______, _______,  KC_ETA,  KC_RHO,  KC_TAU,  KC_UPSILON,KC_THETA, KC_IOTA,KC_OMICRON,KC_PI, _______, _______, _______,
        _______, KC_ALPHA,KC_SIGMA,KC_DELTA, KC_PHI,KC_GAMMA,      KC_ETA,  KC_CHI,KC_KAPPA,KC_LAMBDA,_______, _______, _______,
        _______,  KC_ZETA,  KC_XI,  KC_PSI,KC_OMEGA, KC_BETA,       KC_NU,   KC_MU, _______, _______, _______, _______, _______,
        _______, _______, _______, _______, _______,              _______, _______,          _______, _______, _______, _______

  )
};
// clang-format on

const int notifier_key_to_layer[DYNAMIC_KEYMAP_LAYER_COUNT] = {
    KC_F13, // L0
    KC_F14, // L1
    KC_F15, // L2
};

// Process custom keycodes
// Return
// - false to skip all further processing of this key
// - true to continue processing this key
bool process_record_user(uint16_t keycode, keyrecord_t *record) {

  // Press layer notifier keys during momentary layer switch keys (MO) is
  // pressed
  if (keycode >= MO(1) && keycode <= MO(DYNAMIC_KEYMAP_LAYER_COUNT - 1)) {
    const int layer = QK_MOMENTARY_GET_LAYER(keycode);
    if (layer < 1 || layer >= DYNAMIC_KEYMAP_LAYER_COUNT) {
      return true;
    }

    if (record->event.pressed) {
      register_code(notifier_key_to_layer[layer]);
      layer_on(layer);
    } else {
      unregister_code(notifier_key_to_layer[layer]);
      layer_off(layer);
    }
    return false;
  }

  // Handle Greek letter keycodes
  if (keycode >= KC_ALPHA && keycode <= KC_OMEGA) {
    if (record->event.pressed) {
      static const char *greek_keycode_map[] = {
          "/alpha", "/beta",    "/gamma",   "/delta", "/epsilon", "/zeta",
          "/eta",   "/theta",   "/iota",    "/kappa", "/lambda",  "/mu",
          "/nu",    "/xi",      "/omicron", "/pi",    "/rho",     "/sigma",
          "/tau",   "/upsilon", "/phi",     "/chi",   "/psi",     "/omega"};
      const int greek_index = keycode - KC_ALPHA;
      const int greek_keycode_count =
          (int)(sizeof(greek_keycode_map) / sizeof(greek_keycode_map[0]));
      if (greek_index >= 0 && greek_index < greek_keycode_count) {
        send_string(greek_keycode_map[greek_index]);
      }
    }
    return false;
  }

  return true;
}

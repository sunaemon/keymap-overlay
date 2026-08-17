# Custom Keyboard Configuration

The overlay repository includes two example keyboards, but keyboard-specific
configuration can live in a fork or beside an unmodified submodule. Keeping it
external makes upstream updates easier to pull without mixing them with private
keymaps.

## Directory Layout

```text
keyboard-config/
├── keymap-overlay/           # this repository, as a submodule
└── keyboards/                # your keyboard configuration
    ├── 1/
    └── 2/
```

Seed the directory from the included examples:

```bash
cd ~/keyboard-config
mkdir -p keyboards
cp -R keymap-overlay/firmware/examples/. keyboards/
```

Each directory name is a numeric `KEYBOARD_ID` from 0 through 255. Its
`config.json` names the QMK keyboard used to compile firmware and generate
layer models.

## Encoder Geometry

QMK records encoder pins and order but not the position of each knob in the
physical layout. List encoders in QMK order and identify each push-switch
matrix position:

```json
{
  "qmk_keyboard": "doio/kb16/rev2",
  "encoders": [{ "matrix": [0, 4] }, { "matrix": [1, 4] }, { "matrix": [2, 4] }]
}
```

The generator replaces those push keys with circular controls showing
counter-clockwise, clockwise, and push actions. For an encoder without a push
switch, use explicit QMK layout coordinates such as `{ "x": 4, "y": 0 }`.

## Display Labels

Labels affect generated layer models only; they do not change firmware
behavior.

A one-character trailing comment on a custom keycode becomes its label:

```c
enum custom_keycodes {
  KC_ALPHA = SAFE_RANGE, // α
  KC_BETA,               // β
};
```

Map standard or multi-character labels in a comment block anywhere in
`keymap.c`:

```c
/* keymap-overlay-labels
KC_APP = ☰
KC_LEFT = ←
*/
```

Add `-macos`, `-linux`, or `-windows` for a platform override:

```c
/* keymap-overlay-labels-macos
KC_LGUI = ⌘
KC_LALT = ⌥
*/

/* keymap-overlay-labels-linux
KC_LGUI = Super
KC_LALT = Alt
*/

/* keymap-overlay-labels-windows
KC_LGUI = ⊞
KC_LALT = Alt
*/
```

Asset generation targets the current host by default. Set
`OVERLAY_PLATFORM=windows` when WSL generates models for Windows. These
annotations are ordinary C comments and are consumed only by
`make install-assets`.

## Build and Install

Pass an absolute `KEYBOARDS_DIR`, because `make -C` changes the working
directory:

```bash
cd ~/keyboard-config
make -C keymap-overlay \
  KEYBOARDS_DIR="$PWD/keyboards" \
  flash KEYBOARD_ID=2

make -C keymap-overlay \
  KEYBOARDS_DIR="$PWD/keyboards" \
  install-assets
```

The released binary installer does not change. Every runtime reads the
generated JSON layer models from the platform configuration directory.

Return to the [main README](../README.md#everyday-operations) for update,
restart, upgrade, and uninstall commands.

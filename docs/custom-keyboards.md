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

## Custom Keycode Labels

Labels affect generated layer models only; they do not change firmware
behavior.

Base `enum custom_keycodes` at `QK_KB_0`, Vial's fixed range for a keyboard's
own custom keycodes, and give each entry a one-character trailing comment for
its label:

```c
enum custom_keycodes {
  KC_ALPHA = QK_KB_0, // α
  KC_BETA,            // β
};
```

By default (`VIAL=true`), layer rendering reads a custom keycode's name and
label straight off the connected device's own embedded Vial definition
(`fetch_vial_definition.py`), which `generate_vial.py` embeds from this same
enum at flash time — a base other than `QK_KB_0` desyncs the two, so custom
keycodes render with the wrong label or none at all. `VIAL=false` renders
from `keymap.c` alone, with no device connected; that path also accepts
`SAFE_RANGE`/`QK_USER_0`.

Generic key aliases (arrow glyphs, media keys) and platform-specific ones
(`⌘`/`Super`/`⊞` for the GUI key) are the overlay's own built-in tables in
`model/scripts/generate_overlay_asset.py`, not something `keymap.c`
configures.

Asset generation targets the current host platform by default. Set
`OVERLAY_PLATFORM=windows` when WSL generates models for Windows.

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

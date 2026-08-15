# Copyright 2026 sunaemon
# SPDX-License-Identifier: MIT
import logging
import re
import sys
from io import BytesIO
from pathlib import Path
from textwrap import wrap
from typing import Annotated

import typer
from PIL import Image, ImageDraw, ImageFont

from src.types import (
    EncoderPlacement,
    KeyboardConfig,
    KeyboardJson,
    KeycodesJson,
    LayoutKey,
    QmkKeymapJson,
    VitalyJson,
    parse_json,
)
from src.util import initialize_logging, parse_keycode_value, strip_c_comments

logger = logging.getLogger(__name__)

app = typer.Typer()
Font = ImageFont.ImageFont | ImageFont.FreeTypeFont

BACKGROUND = (0, 0, 0, 0)
KEY_FILL = (248, 248, 242, 255)
KEY_OUTLINE = (68, 71, 90, 255)
KEY_TEXT = (40, 42, 54, 255)
HELD_FILL = (189, 147, 249, 255)
KNOB_FILL = (241, 241, 236, 255)
PADDING = 20
HEADER_HEIGHT = 28
KEY_INSET = 3
SUPERSAMPLE_SCALE = 4
ENCODER_PAIR_NAME = "ENCODER_CCW_CW"
TRANSPARENT_KEYS = {"KC_TRNS", "KC_TRANSPARENT", "_______"}

KEYCODE_LABELS = {
    "KC_NO": "",
    "XXXXXXX": "",
    "KC_AUDIO_MUTE": "MUTE",
    "KC_MUTE": "MUTE",
    "KC_AUDIO_VOL_DOWN": "VOL -",
    "KC_VOLD": "VOL -",
    "KC_AUDIO_VOL_UP": "VOL +",
    "KC_VOLU": "VOL +",
    "KC_MEDIA_PREV_TRACK": "PREV",
    "KC_MPRV": "PREV",
    "KC_MEDIA_NEXT_TRACK": "NEXT",
    "KC_MNXT": "NEXT",
    "KC_MEDIA_PLAY_PAUSE": "PLAY",
    "KC_MPLY": "PLAY",
    "KC_BRIGHTNESS_DOWN": "BRIGHT -",
    "KC_BRID": "BRIGHT -",
    "KC_BRIGHTNESS_UP": "BRIGHT +",
    "KC_BRIU": "BRIGHT +",
    "EIZO_BRIGHTNESS_DOWN": "BRI -",
    "EIZO_BRIGHTNESS_UP": "BRI +",
    "EIZO_USB_C": "EIZO USB-C",
    "EIZO_DP": "EIZO DP",
    "EIZO_PBYP": "EIZO PbyP",
    "QK_BOOT": "QK BOOT",
}


@app.command()
def main(
    qmk_keymap_json: Annotated[Path, typer.Option(help="Raw QMK keymap JSON")],
    keyboard_json: Annotated[Path, typer.Option(help="QMK keyboard.json")],
    keyboard_config: Annotated[Path, typer.Option(help="Project keyboard config.json")],
    custom_keycodes_json: Annotated[
        Path, typer.Option(help="Generated custom keycode mapping")
    ],
    layout_name: Annotated[str, typer.Option(help="Layout to render")],
    layer: Annotated[int, typer.Option(help="Zero-based layer to render")],
    pixels_per_unit: Annotated[
        int, typer.Option(min=32, max=256, help="Pixels per QMK layout unit")
    ] = 64,
    keymap_c: Annotated[
        Path | None, typer.Option(help="keymap.c containing encoder_map")
    ] = None,
    vitaly_json: Annotated[
        Path | None, typer.Option(help="Vitaly dump containing encoder_layout")
    ] = None,
) -> None:
    """Render one keymap layer directly to a transparent PNG on stdout."""
    initialize_logging()
    try:
        image = render_png(
            qmk_keymap_json,
            keyboard_json,
            keyboard_config,
            custom_keycodes_json,
            layout_name,
            layer,
            pixels_per_unit,
            keymap_c=keymap_c,
            vitaly_json=vitaly_json,
        )
        output = BytesIO()
        image.save(output, format="PNG", optimize=True)
        _write_stdout(output.getvalue())
        logger.info("Rendered layer %d from %s", layer, qmk_keymap_json)
    except Exception:
        logger.exception("Failed to render layer %d", layer)
        raise typer.Exit(code=1) from None


def render_png(
    qmk_keymap_json: Path,
    keyboard_json: Path,
    keyboard_config: Path,
    custom_keycodes_json: Path,
    layout_name: str,
    layer_index: int,
    pixels_per_unit: int = 64,
    *,
    keymap_c: Path | None = None,
    vitaly_json: Path | None = None,
) -> Image.Image:
    """Render one keymap layer as an RGBA image."""
    if (keymap_c is None) == (vitaly_json is None):
        raise ValueError("Provide exactly one of keymap_c or vitaly_json")

    keymap = parse_json(QmkKeymapJson, qmk_keymap_json)
    keyboard = parse_json(KeyboardJson, keyboard_json)
    config = parse_json(KeyboardConfig, keyboard_config)
    custom_keycodes = parse_json(KeycodesJson, custom_keycodes_json)
    layout = keyboard.layout_keys(layout_name)
    _validate_layer(keymap, layout, layer_index)

    placements = _resolve_encoder_placements(keyboard, config, layout)
    encoder_layers = _load_encoder_layers(keymap_c, vitaly_json)
    encoder_pairs = _encoder_pairs_for_layer(
        encoder_layers,
        len(placements),
        layer_index,
        custom_keycodes,
    )
    layer = _resolve_layer(keymap, layer_index, custom_keycodes)
    return _draw_layer(
        layout,
        layer,
        placements,
        encoder_pairs,
        layer_index,
        pixels_per_unit,
    )


def _resolve_layer(
    keymap: QmkKeymapJson,
    layer_index: int,
    custom_keycodes: KeycodesJson,
) -> list[str]:
    """Resolve display-only transparency and numeric custom keycodes."""
    layer = list(keymap.layers[layer_index])
    if layer_index > 0:
        base_layer = keymap.layers[0]
        layer = [
            base_layer[index] if keycode in TRANSPARENT_KEYS else keycode
            for index, keycode in enumerate(layer)
        ]
    return [_resolve_custom_keycode(keycode, custom_keycodes) for keycode in layer]


def _validate_layer(
    keymap: QmkKeymapJson,
    layout: list[LayoutKey],
    layer_index: int,
) -> None:
    if layer_index < 0 or layer_index >= len(keymap.layers):
        raise ValueError(f"Layer {layer_index} is not present")
    if len(keymap.layers[layer_index]) != len(layout):
        raise ValueError(
            f"Layer {layer_index} has {len(keymap.layers[layer_index])} keys, layout has {len(layout)}"
        )
    if any(key.r != 0 for key in layout):
        raise ValueError("Rotated QMK layouts are not supported yet")


def _resolve_encoder_placements(
    keyboard: KeyboardJson,
    config: KeyboardConfig,
    layout: list[LayoutKey],
) -> list[tuple[int | None, float, float, float, float]]:
    encoder_count = keyboard.encoder_count()
    if len(config.encoders) != encoder_count:
        raise ValueError(
            f"config.json defines {len(config.encoders)} encoder placements, keyboard.json defines {encoder_count}"
        )

    matrix_to_index = {key.matrix: index for index, key in enumerate(layout)}
    placements = [
        _resolve_encoder_placement(placement, matrix_to_index, layout)
        for placement in config.encoders
    ]
    key_indices = [placement[0] for placement in placements if placement[0] is not None]
    if len(key_indices) != len(set(key_indices)):
        raise ValueError("Multiple encoders use the same matrix position")
    return placements


def _resolve_encoder_placement(
    placement: EncoderPlacement,
    matrix_to_index: dict[tuple[int, int], int],
    layout: list[LayoutKey],
) -> tuple[int | None, float, float, float, float]:
    if placement.matrix is not None:
        if placement.matrix not in matrix_to_index:
            raise ValueError(
                f"Encoder matrix position {placement.matrix} is not in the layout"
            )
        key_index = matrix_to_index[placement.matrix]
        key = layout[key_index]
        return key_index, key.x, key.y, key.w, key.h
    assert placement.x is not None and placement.y is not None
    return None, placement.x, placement.y, 1.0, 1.0


def _load_encoder_layers(
    keymap_c: Path | None,
    vitaly_json: Path | None,
) -> list[list[list[str]]]:
    if vitaly_json is not None:
        return parse_json(VitalyJson, vitaly_json).encoder_layout or []
    assert keymap_c is not None
    return _parse_encoder_map(keymap_c)


def _encoder_pairs_for_layer(
    encoder_layers: list[list[list[str]]],
    encoder_count: int,
    layer_index: int,
    custom_keycodes: KeycodesJson,
) -> list[list[str]]:
    base_pairs = _padded_encoder_pairs(encoder_layers, encoder_count, 0)
    pairs = _padded_encoder_pairs(encoder_layers, encoder_count, layer_index)
    resolved: list[list[str]] = []
    for encoder_index, pair in enumerate(pairs):
        resolved_pair = []
        for direction, keycode in enumerate(pair):
            if layer_index > 0 and keycode in TRANSPARENT_KEYS:
                keycode = base_pairs[encoder_index][direction]
            resolved_pair.append(_resolve_custom_keycode(keycode, custom_keycodes))
        resolved.append(resolved_pair)
    return resolved


def _resolve_custom_keycode(keycode: str, custom_keycodes: KeycodesJson) -> str:
    numeric = parse_keycode_value(keycode)
    if numeric is None:
        return keycode
    return custom_keycodes.root.get(f"0x{numeric:04X}", keycode)


def _padded_encoder_pairs(
    encoder_layers: list[list[list[str]]],
    encoder_count: int,
    layer_index: int,
) -> list[list[str]]:
    pairs = encoder_layers[layer_index] if layer_index < len(encoder_layers) else []
    if len(pairs) > encoder_count:
        raise ValueError(
            f"Layer {layer_index} defines {len(pairs)} encoders, expected at most {encoder_count}"
        )
    output = [list(pair) for pair in pairs]
    if any(len(pair) != 2 for pair in output):
        raise ValueError(
            f"Layer {layer_index} encoder bindings must have two directions"
        )
    output.extend(["KC_NO", "KC_NO"] for _ in range(encoder_count - len(output)))
    return output


def _draw_layer(
    layout: list[LayoutKey],
    layer: list[str],
    placements: list[tuple[int | None, float, float, float, float]],
    encoder_pairs: list[list[str]],
    layer_index: int,
    pixels_per_unit: int,
) -> Image.Image:
    bounds = [(key.x, key.y, key.w, key.h) for key in layout]
    bounds.extend((x, y, width, height) for _, x, y, width, height in placements)
    min_x = min(x for x, _, _, _ in bounds)
    min_y = min(y for _, y, _, _ in bounds)
    max_x = max(x + width for x, _, width, _ in bounds)
    max_y = max(y + height for _, y, _, height in bounds)
    output_size = _canvas_size(min_x, min_y, max_x, max_y, pixels_per_unit, 1)
    render_scale = SUPERSAMPLE_SCALE
    render_pixels_per_unit = pixels_per_unit * render_scale
    width, height = _canvas_size(
        min_x,
        min_y,
        max_x,
        max_y,
        render_pixels_per_unit,
        render_scale,
    )
    image = Image.new("RGBA", (width, height), BACKGROUND)
    draw = ImageDraw.Draw(image)
    fonts = _fonts(render_pixels_per_unit)
    draw.text(
        (PADDING * render_scale, PADDING * render_scale),
        f"L{layer_index}",
        fill=KEY_TEXT,
        font=fonts["header"],
    )

    encoder_key_indices = {
        key_index for key_index, *_ in placements if key_index is not None
    }
    for key_index, key in enumerate(layout):
        if key_index in encoder_key_indices:
            continue
        box = _pixel_box(
            key.x,
            key.y,
            key.w,
            key.h,
            min_x,
            min_y,
            render_pixels_per_unit,
            render_scale,
        )
        _draw_key(draw, box, layer[key_index], layer_index, fonts, render_scale)

    for encoder_index, placement in enumerate(placements):
        key_index, x, y, key_width, key_height = placement
        box = _pixel_box(
            x,
            y,
            key_width,
            key_height,
            min_x,
            min_y,
            render_pixels_per_unit,
            render_scale,
        )
        press = layer[key_index] if key_index is not None else "KC_NO"
        _draw_encoder(
            draw,
            box,
            press,
            encoder_pairs[encoder_index],
            layer_index,
            fonts,
            render_scale,
        )
    return image.resize(output_size, Image.Resampling.LANCZOS)


def _canvas_size(
    min_x: float,
    min_y: float,
    max_x: float,
    max_y: float,
    pixels_per_unit: int,
    render_scale: int,
) -> tuple[int, int]:
    width = round((max_x - min_x) * pixels_per_unit) + 2 * PADDING * render_scale
    height = (
        round((max_y - min_y) * pixels_per_unit)
        + (2 * PADDING + HEADER_HEIGHT) * render_scale
    )
    return width, height


def _pixel_box(
    x: float,
    y: float,
    width: float,
    height: float,
    min_x: float,
    min_y: float,
    pixels_per_unit: int,
    render_scale: int,
) -> tuple[int, int, int, int]:
    left = round((x - min_x) * pixels_per_unit) + PADDING * render_scale
    top = (
        round((y - min_y) * pixels_per_unit) + (PADDING + HEADER_HEIGHT) * render_scale
    )
    return (
        left,
        top,
        left + round(width * pixels_per_unit),
        top + round(height * pixels_per_unit),
    )


def _draw_key(
    draw: ImageDraw.ImageDraw,
    box: tuple[int, int, int, int],
    keycode: str,
    layer_index: int,
    fonts: dict[str, Font],
    render_scale: int,
) -> None:
    held = _momentary_layer(keycode) == layer_index
    inset = _inset_box(box, KEY_INSET * render_scale)
    draw.rounded_rectangle(
        inset,
        radius=7 * render_scale,
        fill=HELD_FILL if held else KEY_FILL,
        outline=KEY_OUTLINE,
        width=2 * render_scale,
    )
    _draw_centered_lines(
        draw, _format_keycode(keycode), _center(inset), fonts["key"], 3
    )


def _draw_encoder(
    draw: ImageDraw.ImageDraw,
    box: tuple[int, int, int, int],
    press_keycode: str,
    directions: list[str],
    layer_index: int,
    fonts: dict[str, Font],
    render_scale: int,
) -> None:
    left, top, right, bottom = _square_box(box)
    held = _momentary_layer(press_keycode) == layer_index
    circle = _inset_box((left, top, right, bottom), 2 * render_scale)
    draw.ellipse(
        circle,
        fill=HELD_FILL if held else KNOB_FILL,
        outline=KEY_OUTLINE,
        width=2 * render_scale,
    )
    center_x, center_y = _center(circle)
    direction_labels = [_format_keycode(keycode) for keycode in directions]
    if direction_labels[0]:
        _draw_turn_marker(
            draw,
            center_x - 14 * render_scale,
            center_y - 19 * render_scale,
            render_scale,
            clockwise=False,
        )
    if direction_labels[1]:
        _draw_turn_marker(
            draw,
            center_x + 14 * render_scale,
            center_y - 19 * render_scale,
            render_scale,
            clockwise=True,
        )
    _draw_centered_lines(
        draw,
        direction_labels[0],
        (center_x - 14 * render_scale, center_y - 3 * render_scale),
        fonts["tiny"],
        2,
        max_chars=5,
    )
    _draw_centered_lines(
        draw,
        direction_labels[1],
        (center_x + 14 * render_scale, center_y - 3 * render_scale),
        fonts["tiny"],
        2,
        max_chars=5,
    )
    press_label = _format_keycode(press_keycode)
    if press_label:
        draw.text(
            (center_x, center_y + 18 * render_scale),
            f"P {press_label}",
            fill=KEY_TEXT,
            font=fonts["tiny"],
            anchor="mm",
        )


def _draw_turn_marker(
    draw: ImageDraw.ImageDraw,
    center_x: int,
    center_y: int,
    render_scale: int,
    *,
    clockwise: bool,
) -> None:
    direction = 1 if clockwise else -1
    draw.line(
        [
            (
                center_x - 4 * direction * render_scale,
                center_y - 3 * render_scale,
            ),
            (center_x, center_y),
            (
                center_x - 4 * direction * render_scale,
                center_y + 3 * render_scale,
            ),
        ],
        fill=KEY_TEXT,
        width=render_scale,
    )


def _fonts(pixels_per_unit: int) -> dict[str, Font]:
    return {
        "header": ImageFont.load_default(size=max(14, pixels_per_unit // 4)),
        "key": ImageFont.load_default(size=max(10, pixels_per_unit // 6)),
        "tiny": ImageFont.load_default(size=max(8, pixels_per_unit // 8)),
    }


def _draw_centered_lines(
    draw: ImageDraw.ImageDraw,
    label: str,
    center: tuple[int, int],
    font: Font,
    max_lines: int,
    *,
    max_chars: int = 10,
) -> None:
    if not label:
        return
    lines = _wrap_label(label, max_lines, max_chars)
    line_height = font.getbbox("Ag")[3] + 1
    start_y = center[1] - (len(lines) - 1) * line_height / 2
    for index, line in enumerate(lines):
        draw.text(
            (center[0], round(start_y + index * line_height)),
            line,
            fill=KEY_TEXT,
            font=font,
            anchor="mm",
        )


def _wrap_label(label: str, max_lines: int, max_chars: int) -> list[str]:
    lines = wrap(label, width=max_chars, break_long_words=True) or [label]
    if len(lines) <= max_lines:
        return lines
    return [*lines[: max_lines - 1], lines[max_lines - 1][: max_chars - 3] + "..."]


def _format_keycode(keycode: str) -> str:
    keycode = keycode.strip()
    if keycode in TRANSPARENT_KEYS:
        return ""
    if keycode in KEYCODE_LABELS:
        return KEYCODE_LABELS[keycode]
    layer = _momentary_layer(keycode)
    if layer is not None:
        return f"L{layer}"
    for prefix in ("KC_", "QK_"):
        if keycode.startswith(prefix):
            keycode = keycode[len(prefix) :]
            break
    return keycode.replace("_", " ")


def _momentary_layer(keycode: str) -> int | None:
    match = re.fullmatch(r"MO\((\d+)\)", keycode.replace(" ", ""))
    return int(match.group(1)) if match else None


def _center(box: tuple[int, int, int, int]) -> tuple[int, int]:
    left, top, right, bottom = box
    return (left + right) // 2, (top + bottom) // 2


def _inset_box(
    box: tuple[int, int, int, int],
    inset: int,
) -> tuple[int, int, int, int]:
    left, top, right, bottom = box
    return left + inset, top + inset, right - inset, bottom - inset


def _square_box(box: tuple[int, int, int, int]) -> tuple[int, int, int, int]:
    left, top, right, bottom = box
    size = min(right - left, bottom - top)
    center_x, center_y = _center(box)
    half = size // 2
    return center_x - half, center_y - half, center_x + half, center_y + half


def _parse_encoder_map(keymap_c: Path) -> list[list[list[str]]]:
    content = strip_c_comments(keymap_c.read_text(encoding="utf-8"))
    name = re.search(r"\bencoder_map\b", content)
    if name is None:
        return []
    equals = content.find("=", name.end())
    opening = content.find("{", equals + 1)
    if equals < 0 or opening < 0:
        raise ValueError(f"Malformed encoder_map in {keymap_c}")
    body, _ = _extract_delimited(content, opening, "{", "}")
    return _parse_encoder_layers(body, keymap_c)


def _parse_encoder_layers(content: str, keymap_c: Path) -> list[list[list[str]]]:
    indexed_layers: dict[int, list[list[str]]] = {}
    position = 0
    layer_pattern = re.compile(r"\[(\d+)\]\s*=")
    while match := layer_pattern.search(content, position):
        opening = content.find("{", match.end())
        if opening < 0:
            raise ValueError(f"Malformed encoder_map layer in {keymap_c}")
        body, position = _extract_delimited(content, opening, "{", "}")
        layer_index = int(match.group(1))
        if layer_index in indexed_layers:
            raise ValueError(f"Duplicate encoder_map layer {layer_index} in {keymap_c}")
        indexed_layers[layer_index] = _parse_encoder_pairs(body, keymap_c)
    if not indexed_layers:
        return []
    layers = [[] for _ in range(max(indexed_layers) + 1)]
    for layer_index, pairs in indexed_layers.items():
        layers[layer_index] = pairs
    return layers


def _parse_encoder_pairs(content: str, keymap_c: Path) -> list[list[str]]:
    pairs: list[list[str]] = []
    position = 0
    while (start := content.find(ENCODER_PAIR_NAME, position)) >= 0:
        opening = content.find("(", start + len(ENCODER_PAIR_NAME))
        if opening < 0:
            raise ValueError(f"Malformed {ENCODER_PAIR_NAME} in {keymap_c}")
        arguments, position = _extract_delimited(content, opening, "(", ")")
        pairs.append(_split_pair(arguments, keymap_c))
    return pairs


def _split_pair(arguments: str, keymap_c: Path) -> list[str]:
    depth = 0
    separators = []
    for index, character in enumerate(arguments):
        if character == "(":
            depth += 1
        elif character == ")":
            depth -= 1
        elif character == "," and depth == 0:
            separators.append(index)
    if len(separators) != 1 or depth != 0:
        raise ValueError(f"Malformed {ENCODER_PAIR_NAME} arguments in {keymap_c}")
    separator = separators[0]
    return [arguments[:separator].strip(), arguments[separator + 1 :].strip()]


def _extract_delimited(
    content: str,
    opening_index: int,
    opening_character: str,
    closing_character: str,
) -> tuple[str, int]:
    depth = 0
    for index in range(opening_index, len(content)):
        character = content[index]
        if character == opening_character:
            depth += 1
        elif character == closing_character:
            depth -= 1
            if depth == 0:
                return content[opening_index + 1 : index], index + 1
    raise ValueError(f"Unclosed {opening_character} in encoder_map")


def _write_stdout(data: bytes) -> None:
    buffer = getattr(sys.stdout, "buffer", None)
    if buffer is None:
        raise OSError("Binary stdout is unavailable")
    sys.stdout.flush()
    buffer.write(data)
    buffer.flush()


if __name__ == "__main__":
    app()

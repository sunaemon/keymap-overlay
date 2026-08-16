# Copyright 2026 sunaemon
# SPDX-License-Identifier: MIT
import ast
import operator
import re
from collections.abc import Callable
from pathlib import Path

from model.src.util import strip_c_comments

ENCODER_PAIR_NAME = "ENCODER_CCW_CW"
C_INTEGER_SUFFIX = re.compile(
    r"(?i)\b(0x[0-9a-f]+|0b[01]+|[0-9]+)(?:u(?:ll|l)?|(?:ll|l)u?)\b"
)

BINARY_INTEGER_OPERATORS: dict[type[ast.operator], Callable[[int, int], int]] = {
    ast.Add: operator.add,
    ast.Sub: operator.sub,
    ast.Mult: operator.mul,
    ast.Div: lambda left, right: _c_div(left, right),
    ast.Mod: lambda left, right: left - _c_div(left, right) * right,
    ast.LShift: operator.lshift,
    ast.RShift: operator.rshift,
    ast.BitOr: operator.or_,
    ast.BitXor: operator.xor,
    ast.BitAnd: operator.and_,
}
UNARY_INTEGER_OPERATORS: dict[type[ast.unaryop], Callable[[int], int]] = {
    ast.UAdd: operator.pos,
    ast.USub: operator.neg,
    ast.Invert: operator.invert,
}


def parse_encoder_map(keymap_c: Path) -> list[list[list[str]]]:
    """Parse QMK encoder bindings from keymap.c."""
    content = strip_c_comments(keymap_c.read_text(encoding="utf-8"))
    name = re.search(r"\bencoder_map\b", content)
    if name is None:
        return []
    equals = content.find("=", name.end())
    opening = content.find("{", equals + 1)
    if equals < 0 or opening < 0:
        raise ValueError(f"Malformed encoder_map in {keymap_c}")
    body, _ = _extract_delimited(content, opening, "{", "}")
    return _parse_encoder_layers(body, _parse_enum_values(content), keymap_c)


def _parse_encoder_layers(
    content: str, layer_names: dict[str, int], keymap_c: Path
) -> list[list[list[str]]]:
    """Parse each designated encoder layer into its numeric position."""
    indexed_layers: dict[int, list[list[str]]] = {}
    position = 0
    layer_pattern = re.compile(r"\[([A-Za-z_]\w*|\d+)\]\s*=")
    while match := layer_pattern.search(content, position):
        opening = content.find("{", match.end())
        if opening < 0:
            raise ValueError(f"Malformed encoder_map layer in {keymap_c}")
        body, position = _extract_delimited(content, opening, "{", "}")
        designator = match.group(1)
        if designator.isdigit():
            layer_index = int(designator)
        elif designator in layer_names:
            layer_index = layer_names[designator]
        else:
            raise ValueError(
                f"Unknown encoder_map layer designator {designator} in {keymap_c}"
            )
        if layer_index in indexed_layers:
            raise ValueError(f"Duplicate encoder_map layer {layer_index} in {keymap_c}")
        indexed_layers[layer_index] = _parse_encoder_pairs(body, keymap_c)
    if not indexed_layers:
        raise ValueError(f"encoder_map has no layer designators in {keymap_c}")
    layers = [[] for _ in range(max(indexed_layers) + 1)]
    for layer_index, pairs in indexed_layers.items():
        layers[layer_index] = pairs
    return layers


def _parse_encoder_pairs(content: str, keymap_c: Path) -> list[list[str]]:
    """Extract counter-clockwise and clockwise action pairs from one layer."""
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
    """Split one encoder macro's two potentially nested arguments."""
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
    pair = [arguments[:separator].strip(), arguments[separator + 1 :].strip()]
    if not all(pair):
        raise ValueError(f"Empty {ENCODER_PAIR_NAME} argument in {keymap_c}")
    return pair


def _parse_enum_values(content: str) -> dict[str, int]:
    """Resolve integer values for C enumerators used as layer designators."""
    values: dict[str, int] = {}
    enum_pattern = re.compile(r"\benum(?:\s+[A-Za-z_]\w*)?\s*\{([^}]*)\}", re.DOTALL)
    for match in enum_pattern.finditer(content):
        current: int | None = -1
        for raw_entry in match.group(1).split(","):
            entry = raw_entry.strip()
            if not entry:
                continue
            name, separator, raw_value = entry.partition("=")
            name = name.strip()
            if not re.fullmatch(r"[A-Za-z_]\w*", name):
                current = None
                continue
            if separator:
                current = _evaluate_integer_expression(raw_value.strip(), values)
            elif current is not None:
                current += 1
            if current is not None:
                values[name] = current
    return values


def _evaluate_integer_expression(expression: str, values: dict[str, int]) -> int | None:
    """Evaluate the integer-only subset of C used by enum initializers."""
    expression = C_INTEGER_SUFFIX.sub(r"\1", expression)
    try:
        parsed = ast.parse(expression, mode="eval")
        return _evaluate_integer_node(parsed.body, values)
    except (KeyError, SyntaxError, TypeError, ValueError, ZeroDivisionError):
        return None


def _evaluate_integer_node(node: ast.expr, values: dict[str, int]) -> int:
    """Evaluate one validated integer-expression node."""
    if isinstance(node, ast.Constant) and type(node.value) is int:
        return node.value
    if isinstance(node, ast.Name):
        return values[node.id]
    if isinstance(node, ast.BinOp):
        operation = BINARY_INTEGER_OPERATORS[type(node.op)]
        return operation(
            _evaluate_integer_node(node.left, values),
            _evaluate_integer_node(node.right, values),
        )
    if isinstance(node, ast.UnaryOp):
        operation = UNARY_INTEGER_OPERATORS[type(node.op)]
        return operation(_evaluate_integer_node(node.operand, values))
    raise ValueError("unsupported integer expression")


def _c_div(left: int, right: int) -> int:
    """Divide integers with C's truncation toward zero."""
    quotient = abs(left) // abs(right)
    return -quotient if (left < 0) != (right < 0) else quotient


def _extract_delimited(
    content: str,
    opening_index: int,
    opening_character: str,
    closing_character: str,
) -> tuple[str, int]:
    """Return text within one balanced delimiter pair and its ending offset."""
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

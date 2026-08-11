# Copyright 2026 sunaemon
# SPDX-License-Identifier: MIT
import logging
from pathlib import Path
from typing import Annotated

import typer

from src.util import initialize_logging

logger = logging.getLogger(__name__)

app = typer.Typer()

SUBJECT_MAX_LENGTH = 72

# Git writes these subjects itself, so their shape is not ours to enforce.
# `Reapply ` is what a revert of a revert produces.
GENERATED_SUBJECT_PREFIXES = ("Merge ", "Revert ", "Reapply ", "fixup!", "squash!")

# `git commit --verbose` appends the diff below a comment holding this marker.
SCISSORS_MARKER = ">8"


@app.command()
def main(
    commit_msg_file: Annotated[
        Path, typer.Argument(help="Path to the file holding the commit message")
    ],
) -> None:
    """Check a commit message and exit non-zero when it needs work."""
    initialize_logging()
    # Git writes the message as UTF-8; decoding with the locale encoding would
    # crash or miscount characters under a non-UTF-8 LANG.
    try:
        message = commit_msg_file.read_text(encoding="utf-8")
    except (OSError, UnicodeDecodeError):
        logger.exception("Failed to read the commit message from %s", commit_msg_file)
        raise typer.Exit(code=1) from None

    problems = check_commit_message(message)
    if problems:
        for problem in problems:
            logger.error("%s", problem)
        raise typer.Exit(code=1)


def check_commit_message(message: str) -> list[str]:
    """Return every problem found in a commit message, empty when it is fine."""
    lines = _message_lines(message)
    if not lines:
        return ["The commit message is empty."]

    subject = lines[0]
    if subject.startswith(GENERATED_SUBJECT_PREFIXES):
        return []

    problems: list[str] = []
    if len(subject) > SUBJECT_MAX_LENGTH:
        problems.append(
            f"The subject is {len(subject)} characters long; "
            f"keep it within {SUBJECT_MAX_LENGTH}."
        )
    if subject.endswith("."):
        problems.append("The subject ends with a period; drop it.")
    if len(lines) > 1 and lines[1].strip():
        problems.append("Leave a blank line between the subject and the body.")
    return problems


def _message_lines(message: str) -> list[str]:
    """Return the lines git will keep, without comments or surrounding blanks."""
    lines: list[str] = []
    for line in message.splitlines():
        if line.startswith("#"):
            # Everything below the scissors is the diff, not the message.
            if SCISSORS_MARKER in line:
                break
            continue
        lines.append(line)

    while lines and not lines[0].strip():
        lines.pop(0)
    while lines and not lines[-1].strip():
        lines.pop()
    return lines


if __name__ == "__main__":
    app()

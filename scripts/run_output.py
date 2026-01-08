# Copyright 2025 sunaemon
# SPDX-License-Identifier: MIT
import logging
import subprocess
from pathlib import Path
from typing import Annotated

import typer

from src.util import initialize_logging

logger = logging.getLogger(__name__)

app = typer.Typer()


@app.command()
def main(
    output: Annotated[Path, typer.Argument(help="Final output path")],
    command: Annotated[
        list[str], typer.Argument(help="Command to run; use -- before it")
    ],
) -> None:
    """Run a command and atomically write its stdout to the output path."""
    initialize_logging()
    try:
        _run_output(output, command)
    except Exception:
        logger.exception("Failed to run command for %s", output)
        raise typer.Exit(code=1) from None


def _run_output(output: Path, command: list[str]) -> None:
    if not command:
        raise ValueError("Command is required")

    output.parent.mkdir(parents=True, exist_ok=True)
    tmp_path = output.with_suffix(output.suffix + ".tmp")

    try:
        with tmp_path.open("w") as handle:
            result = subprocess.run(
                command, stdout=handle, stderr=subprocess.PIPE, text=True, check=False
            )
        if result.returncode != 0:
            stderr = (result.stderr or "").strip()
            if stderr:
                logger.error("Command stderr: %s", stderr)
                raise RuntimeError(
                    f"Command failed with exit code {result.returncode}: {stderr}"
                )
            raise RuntimeError(f"Command failed with exit code {result.returncode}")
        tmp_path.replace(output)
    except Exception:
        tmp_path.unlink(missing_ok=True)
        raise


if __name__ == "__main__":
    app()

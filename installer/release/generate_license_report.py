# Copyright 2026 sunaemon
# SPDX-License-Identifier: MIT
import difflib
import logging
import subprocess
import tempfile
from collections.abc import Callable
from pathlib import Path
from typing import Annotated

import typer

from model.src.util import initialize_logging

logger = logging.getLogger(__name__)

app = typer.Typer()

CommandRunner = Callable[[list[str]], subprocess.CompletedProcess[str]]

DEFAULT_OUTPUT = Path("THIRD-PARTY-LICENSES.html")
DEFAULT_CONFIG = Path("docs/about.toml")
DEFAULT_TEMPLATE = Path("docs/third-party-licenses.hbs")


class LicenseReportDriftError(Exception):
    """Raised when the checked-in license report is stale."""


@app.command()
def main(
    check: Annotated[
        bool, typer.Option(help="Check the report without changing it")
    ] = False,
    output: Annotated[
        Path, typer.Option(help="Generated third-party license report")
    ] = DEFAULT_OUTPUT,
    config: Annotated[
        Path, typer.Option(help="cargo-about configuration")
    ] = DEFAULT_CONFIG,
    template: Annotated[
        Path, typer.Option(help="cargo-about Handlebars template")
    ] = DEFAULT_TEMPLATE,
) -> None:
    """Generate or check the third-party license report."""
    initialize_logging()
    try:
        report = generate_license_report(template, config=config)
        if check:
            check_license_report(output, report)
        else:
            output.write_text(report, encoding="utf-8")
            logger.info("Generated %s", output)
    except (LicenseReportDriftError, OSError, subprocess.SubprocessError):
        logger.exception("Failed to %s %s", "check" if check else "generate", output)
        raise typer.Exit(code=1) from None


def generate_license_report(
    template: Path,
    *,
    config: Path = DEFAULT_CONFIG,
    runner: CommandRunner | None = None,
) -> str:
    """Generate and normalize the Cargo workspace license report."""
    run = runner or run_command
    with tempfile.TemporaryDirectory() as temporary_directory:
        raw_report = Path(temporary_directory) / "license-report.html"
        run(
            [
                "cargo-about",
                "generate",
                str(template),
                "--config",
                str(config),
                "--workspace",
                "--all-features",
                "--locked",
                "--fail",
                "--output-file",
                str(raw_report),
            ]
        )
        return normalize_report(raw_report.read_text(encoding="utf-8"))


def check_license_report(path: Path, expected: str) -> None:
    """Raise with a concise diff when a checked-in report is stale."""
    actual = path.read_text(encoding="utf-8")
    if actual == expected:
        logger.info("%s is current", path)
        return

    difference = "".join(
        difflib.unified_diff(
            actual.splitlines(keepends=True),
            expected.splitlines(keepends=True),
            fromfile=str(path),
            tofile="generated license report",
        )
    )
    raise LicenseReportDriftError(
        f"{path} is stale; run `make licenses` and stage the result.\n{difference}"
    )


def normalize_report(report: str) -> str:
    """Remove trailing whitespace from every generated report line."""
    return "\n".join(line.rstrip() for line in report.split("\n"))


def run_command(command: list[str]) -> subprocess.CompletedProcess[str]:
    """Run cargo-about and capture its text output."""
    return subprocess.run(command, check=True, capture_output=True, text=True)


if __name__ == "__main__":
    app()

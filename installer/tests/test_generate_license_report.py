# Copyright 2026 sunaemon
# SPDX-License-Identifier: MIT
import subprocess
from pathlib import Path

import pytest

from installer.release.generate_license_report import (
    LicenseReportDriftError,
    check_license_report,
    generate_license_report,
    normalize_report,
)


def test_generate_report_invokes_cargo_about_and_normalizes_output(
    tmp_path: Path,
) -> None:
    commands: list[list[str]] = []
    template = tmp_path / "licenses.hbs"
    template.write_text("template")
    config = tmp_path / "about.toml"
    config.write_text("accepted = []")

    def capture_command(command: list[str]) -> subprocess.CompletedProcess[str]:
        commands.append(command)
        return report_runner(command)

    report = generate_license_report(template, config=config, runner=capture_command)

    assert report == "first line\nsecond line\n"
    assert commands[0][commands[0].index("--config") + 1] == str(config)


def test_generate_report_accepts_a_standalone_manifest(tmp_path: Path) -> None:
    """Pass a standalone Cargo manifest to cargo-about."""
    commands: list[list[str]] = []
    template = tmp_path / "licenses.hbs"
    template.write_text("template")
    config = tmp_path / "about.toml"
    config.write_text("accepted = []")
    manifest = tmp_path / "standalone/Cargo.toml"

    def capture_command(command: list[str]) -> subprocess.CompletedProcess[str]:
        """Record the cargo-about command before returning its test report."""
        commands.append(command)
        return report_runner(command)

    generate_license_report(
        template,
        config=config,
        manifest=manifest,
        runner=capture_command,
    )

    assert commands[0][commands[0].index("--manifest-path") + 1] == str(manifest)


def test_current_report_passes_the_check(tmp_path: Path) -> None:
    report = tmp_path / "THIRD-PARTY-LICENSES.html"
    report.write_text("current\n")

    check_license_report(report, "current\n")


def test_stale_report_explains_how_to_regenerate_it(tmp_path: Path) -> None:
    report = tmp_path / "THIRD-PARTY-LICENSES.html"
    report.write_text("old\n")

    with pytest.raises(LicenseReportDriftError, match="make licenses") as error:
        check_license_report(report, "new\n")

    assert "-old" in str(error.value)
    assert "+new" in str(error.value)


def test_normalize_report_removes_all_trailing_whitespace() -> None:
    assert normalize_report("first  \nsecond\t\n") == "first\nsecond\n"


def test_normalize_report_preserves_a_missing_final_newline() -> None:
    assert normalize_report("first  \nsecond\t") == "first\nsecond"


def report_runner(command: list[str]) -> subprocess.CompletedProcess[str]:
    """Write a raw cargo-about report to its requested output path."""
    output = Path(command[command.index("--output-file") + 1])
    output.write_text("first line  \nsecond line\t\n")
    return subprocess.CompletedProcess(command, 0, "", "")

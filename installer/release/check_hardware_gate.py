# Copyright 2026 sunaemon
# SPDX-License-Identifier: MIT
import json
import logging
import re
import subprocess
import tomllib
from collections.abc import Callable
from dataclasses import dataclass
from pathlib import Path
from typing import Annotated

import typer
from pydantic import BaseModel, TypeAdapter, ValidationError

from model.src.types import KeyboardConfig
from model.src.util import initialize_logging

logger = logging.getLogger(__name__)

app = typer.Typer()

CommandRunner = Callable[[list[str]], subprocess.CompletedProcess[str]]

EXPECTED_PLATFORM_IDS = (
    "macos-arm64-appkit",
    "linux-x86_64-kde-wayland",
    "linux-x86_64-gnome-wayland",
    "windows-x86_64-wpf",
)
PLATFORM_ARCHITECTURES = {
    "macos-arm64-appkit": "arm64",
    "linux-x86_64-kde-wayland": "x86_64",
    "linux-x86_64-gnome-wayland": "x86_64",
    "windows-x86_64-wpf": "x86_64",
}
EXPECTED_GLOBAL_CHECK_IDS = ("GLOBAL-01", "GLOBAL-02")
EXPECTED_CHECK_SECTIONS = {
    "Platform-independent checks": EXPECTED_GLOBAL_CHECK_IDS,
    "macos-arm64-appkit checks": tuple(f"MAC-{number:02d}" for number in range(1, 11)),
    "linux-x86_64 shared checks": tuple(
        f"LX-{number:02d}" for number in (2, 3, 7, 8, 9)
    ),
    "linux-x86_64-kde-wayland checks": tuple(
        f"KDE-{number:02d}" for number in (1, 4, 5, 6, 10)
    ),
    "linux-x86_64-gnome-wayland checks": tuple(
        f"GNOME-{number:02d}" for number in (1, 4, 5, 6, 10)
    ),
    "windows-x86_64-wpf checks": tuple(f"WIN-{number:02d}" for number in range(1, 11)),
}
EXPECTED_CHECK_IDS = tuple(
    check_id for check_ids in EXPECTED_CHECK_SECTIONS.values() for check_id in check_ids
)
EXPECTED_COVERAGE_IDS = (
    "bundled-keyboards",
    "encoder-keyboard",
    "simultaneous-keyboards",
)
EXPECTED_LIFECYCLE_IDS = (
    "macos-arm64-appkit",
    "linux-x86_64",
    "windows-x86_64-wpf",
)
CONDITIONAL_CHECK_IDS = frozenset(EXPECTED_GLOBAL_CHECK_IDS)
FIRMWARE_EVIDENCE_PATHS = frozenset({"Makefile"})
FIRMWARE_EVIDENCE_PREFIXES = ("firmware/", "model/")
CANDIDATE_PATTERN = re.compile(
    r"^Candidate commit:\s*`([0-9a-fA-F]{40})`\s*$", re.MULTILINE
)
CHECK_PATTERN = re.compile(
    r"^- \[([ xX])] \*\*([A-Z]+-[0-9]{2})\*\* — Result: ([^—\n]+?) —",
    re.MULTILINE,
)
PLACEHOLDER_VALUES = frozenset({"", "-", "n/a", "none", "pending", "tbd"})


class HardwareGateError(Exception):
    """Raised when a release PR has incomplete or stale hardware evidence."""


class PullRequestRef(BaseModel):
    """A Git reference in the pull request event."""

    sha: str


class PullRequest(BaseModel):
    """The pull request fields needed by the hardware gate."""

    body: str | None
    base: PullRequestRef
    head: PullRequestRef


class PullRequestEvent(BaseModel):
    """The GitHub pull request event fields needed by the hardware gate."""

    pull_request: PullRequest


@dataclass(frozen=True)
class KeyboardRequirements:
    """Bundled keyboard IDs and the subset that has encoders."""

    bundled_ids: frozenset[int]
    encoder_ids: frozenset[int]


PULL_REQUEST_EVENT_ADAPTER = TypeAdapter(PullRequestEvent)


@app.command()
def main(
    event: Annotated[
        Path, typer.Option(help="Path to the GitHub pull request event JSON")
    ],
    project_file: Annotated[
        Path, typer.Option(help="Current release metadata file")
    ] = Path("pyproject.toml"),
) -> None:
    """Require current hardware evidence when a pull request bumps the version."""
    initialize_logging()
    try:
        check_pull_request_event(event, project_file=project_file)
    except (HardwareGateError, OSError, subprocess.SubprocessError):
        logger.exception("Hardware release gate failed")
        raise typer.Exit(code=1) from None


def check_pull_request_event(
    event_path: Path,
    *,
    project_file: Path = Path("pyproject.toml"),
    runner: CommandRunner | None = None,
) -> None:
    """Validate the hardware gate when the pull request changes the version."""
    event = _read_pull_request_event(event_path)
    current_version = _read_project_version(project_file.read_bytes())
    run = runner or run_command
    previous_project = run(
        ["git", "show", f"{event.pull_request.base.sha}:pyproject.toml"]
    ).stdout.encode()
    previous_version = _read_project_version(previous_project)
    if current_version == previous_version:
        logger.info("The pull request does not change the version; skipping the gate")
        return

    validate_hardware_gate(
        event.pull_request.body,
        event.pull_request.head.sha,
        parse_changed_paths(
            run(
                changed_files_command(
                    f"v{previous_version}",
                    event.pull_request.head.sha,
                )
            ).stdout
        ),
    )
    logger.info("Hardware release gate passes for %s", event.pull_request.head.sha)


def validate_hardware_gate(
    body: str | None,
    head_sha: str,
    changed_paths: frozenset[str],
    *,
    keyboards_directory: Path = Path("firmware/examples"),
) -> None:
    """Require complete evidence for the exact pull request head."""
    evidence = body or ""
    problems: list[str] = []

    _validate_candidate(evidence, head_sha, problems)
    _validate_checklist(evidence, changed_paths, problems)
    requirements = _read_keyboard_requirements(keyboards_directory)
    platform_keyboards = _validate_platform_matrix(evidence, requirements, problems)
    _validate_coverage_matrix(evidence, requirements, platform_keyboards, problems)
    _validate_lifecycle_matrix(evidence, problems)

    covered_ids = (
        set().union(*platform_keyboards.values()) if platform_keyboards else set()
    )
    missing_keyboard_ids = sorted(requirements.bundled_ids - covered_ids)
    if missing_keyboard_ids:
        problems.append(
            "platform matrix does not cover bundled KEYBOARD_ID(s): "
            f"{', '.join(map(str, missing_keyboard_ids))}"
        )

    if problems:
        raise HardwareGateError("; ".join(problems))


def run_command(command: list[str]) -> subprocess.CompletedProcess[str]:
    """Run one gate command and capture its text output."""
    return subprocess.run(command, check=True, capture_output=True, text=True)


def changed_files_command(base_ref: str, head_sha: str) -> list[str]:
    """Return the command that lists paths changed between two revisions."""
    return [
        "git",
        "diff",
        "--no-renames",
        "--name-only",
        "-z",
        base_ref,
        head_sha,
        "--",
    ]


def parse_changed_paths(output: str) -> frozenset[str]:
    """Parse NUL-delimited paths emitted by git diff."""
    return frozenset(path for path in output.split("\0") if path)


def _validate_candidate(evidence: str, head_sha: str, problems: list[str]) -> None:
    """Require one candidate line matching the pull request head."""
    candidates = CANDIDATE_PATTERN.findall(evidence)
    if len(candidates) != 1:
        problems.append(
            "include exactly one Candidate commit line with a backticked "
            "40-character SHA"
        )
    elif candidates[0].lower() != head_sha.lower():
        problems.append(
            f"candidate commit {candidates[0]} does not match PR head {head_sha}"
        )


def _validate_checklist(
    evidence: str, changed_paths: frozenset[str], problems: list[str]
) -> None:
    """Require global and platform checks in their explicit sections."""
    checks: dict[str, bool] = {}
    results: dict[str, str] = {}
    duplicates: list[str] = []
    for heading, expected_ids in EXPECTED_CHECK_SECTIONS.items():
        section = _read_section(evidence, heading, problems)
        section_checks = CHECK_PATTERN.findall(section)
        section_ids = {check_id for _, check_id, _ in section_checks}
        misplaced = sorted(section_ids - set(expected_ids))
        if misplaced:
            problems.append(
                f"{heading} contains misplaced checks: {', '.join(misplaced)}"
            )
        for state, check_id, result in section_checks:
            if check_id in checks:
                duplicates.append(check_id)
            checks[check_id] = state.lower() == "x"
            results[check_id] = result.strip()

    expected = set(EXPECTED_CHECK_IDS)
    missing = sorted(expected - checks.keys())
    unknown = sorted(checks.keys() - expected)
    unchecked = sorted(check_id for check_id in expected if not checks.get(check_id))
    if missing:
        problems.append(f"missing checks: {', '.join(missing)}")
    if unknown:
        problems.append(f"unknown checks: {', '.join(unknown)}")
    if duplicates:
        problems.append(f"duplicate checks: {', '.join(sorted(set(duplicates)))}")
    if unchecked:
        problems.append(f"unchecked checks: {', '.join(unchecked)}")

    invalid_results = sorted(
        check_id
        for check_id in expected
        if checks.get(check_id)
        and not _valid_check_result(
            check_id,
            results.get(check_id, ""),
            changed_paths,
        )
    )
    if invalid_results:
        problems.append("invalid or unreasoned results: " + ", ".join(invalid_results))


def _valid_check_result(
    check_id: str, result: str, changed_paths: frozenset[str]
) -> bool:
    """Return whether a result is PASS or an allowed reasoned N/A."""
    if result == "PASS":
        return True
    prefix = "N/A:"
    return (
        check_id in CONDITIONAL_CHECK_IDS
        and not _requires_firmware_evidence(changed_paths)
        and result.startswith(prefix)
        and not _is_placeholder(result.removeprefix(prefix))
    )


def _requires_firmware_evidence(changed_paths: frozenset[str]) -> bool:
    """Return whether candidate changes require flash and EEPROM evidence."""
    return any(
        path in FIRMWARE_EVIDENCE_PATHS or path.startswith(FIRMWARE_EVIDENCE_PREFIXES)
        for path in changed_paths
    )


def _read_keyboard_requirements(directory: Path) -> KeyboardRequirements:
    """Read bundled and encoder keyboard IDs from release definitions."""
    bundled_ids: set[int] = set()
    encoder_ids: set[int] = set()
    try:
        definitions = sorted(
            path
            for path in directory.iterdir()
            if path.is_dir() and path.name.isdigit()
        )
        for definition in definitions:
            keyboard_id = int(definition.name)
            if not 0 <= keyboard_id <= 255:
                raise HardwareGateError(
                    f"Bundled KEYBOARD_ID is outside 0..255: {keyboard_id}"
                )
            config = _read_keyboard_config(definition / "config.json")
            bundled_ids.add(keyboard_id)
            if config.encoders:
                encoder_ids.add(keyboard_id)
    except OSError as error:
        raise HardwareGateError(
            f"Cannot read bundled keyboard definitions: {directory}"
        ) from error

    if not bundled_ids:
        raise HardwareGateError(f"No bundled keyboard definitions found in {directory}")
    if not encoder_ids:
        raise HardwareGateError(f"No encoder keyboard definition found in {directory}")
    return KeyboardRequirements(frozenset(bundled_ids), frozenset(encoder_ids))


def _read_keyboard_config(path: Path) -> KeyboardConfig:
    """Read and validate one bundled keyboard configuration."""
    try:
        data = json.loads(path.read_text(encoding="utf-8"))
        return KeyboardConfig.model_validate(data)
    except (OSError, json.JSONDecodeError, ValidationError) as error:
        raise HardwareGateError(f"Invalid bundled keyboard config: {path}") from error


def _validate_platform_matrix(
    evidence: str,
    requirements: KeyboardRequirements,
    problems: list[str],
) -> dict[str, set[int]]:
    """Validate required platform rows and return their keyboard IDs."""
    rows = _read_table(
        evidence,
        "Platform test matrix",
        (
            "Platform ID",
            "Architecture",
            "OS version",
            "Desktop / session",
            "Keyboard(s)",
            "KEYBOARD_ID(s)",
            "Firmware revision(s)",
        ),
        problems,
    )
    indexed = _index_rows(rows, EXPECTED_PLATFORM_IDS, "platform", problems)
    keyboard_ids: dict[str, set[int]] = {}
    for platform_id, row in indexed.items():
        if row[1] != PLATFORM_ARCHITECTURES[platform_id]:
            problems.append(
                f"{platform_id} architecture must be "
                f"{PLATFORM_ARCHITECTURES[platform_id]}"
            )
        for column, value in zip(
            ("OS version", "desktop / session", "keyboard", "firmware revision"),
            (row[2], row[3], row[4], row[6]),
            strict=True,
        ):
            if _is_placeholder(value):
                problems.append(f"{platform_id} has no {column}")
        keyboard_ids[platform_id] = _parse_keyboard_ids(
            row[5], f"{platform_id} platform row", problems
        )
    known_ids = set(requirements.bundled_ids)
    for platform_id, row_ids in keyboard_ids.items():
        unknown_ids = sorted(row_ids - known_ids)
        if unknown_ids:
            problems.append(
                f"{platform_id} lists unbundled KEYBOARD_ID(s): "
                f"{', '.join(map(str, unknown_ids))}"
            )
    return keyboard_ids


def _validate_coverage_matrix(
    evidence: str,
    requirements: KeyboardRequirements,
    platform_keyboards: dict[str, set[int]],
    problems: list[str],
) -> None:
    """Validate bundled, encoder, and simultaneous keyboard evidence."""
    rows = _read_table(
        evidence,
        "Keyboard coverage",
        (
            "Coverage ID",
            "Keyboard(s)",
            "KEYBOARD_ID(s)",
            "Platform ID(s)",
            "Result",
        ),
        problems,
    )
    indexed = _index_rows(rows, EXPECTED_COVERAGE_IDS, "coverage", problems)
    coverage_ids, coverage_platforms = _parse_coverage_rows(indexed, problems)
    _validate_coverage_assignments(
        coverage_ids, coverage_platforms, platform_keyboards, problems
    )
    _validate_required_coverage(
        requirements, coverage_ids, coverage_platforms, platform_keyboards, problems
    )


def _parse_coverage_rows(
    indexed: dict[str, list[str]], problems: list[str]
) -> tuple[dict[str, set[int]], dict[str, set[str]]]:
    """Parse keyboard and platform IDs from indexed coverage rows."""
    coverage_ids: dict[str, set[int]] = {}
    coverage_platforms: dict[str, set[str]] = {}
    for coverage_id, row in indexed.items():
        if _is_placeholder(row[1]):
            problems.append(f"{coverage_id} has no keyboard name")
        coverage_ids[coverage_id] = _parse_keyboard_ids(
            row[2], f"{coverage_id} coverage row", problems
        )
        coverage_platforms[coverage_id] = _parse_platform_ids(
            row[3], f"{coverage_id} coverage row", problems
        )
        if row[4] != "PASS":
            problems.append(f"{coverage_id} result must be PASS")
    return coverage_ids, coverage_platforms


def _validate_coverage_assignments(
    coverage_ids: dict[str, set[int]],
    coverage_platforms: dict[str, set[str]],
    platform_keyboards: dict[str, set[int]],
    problems: list[str],
) -> None:
    """Require coverage keyboards to appear in their assigned platform rows."""

    for coverage_id, keyboard_ids in coverage_ids.items():
        assigned_platforms = coverage_platforms.get(coverage_id, set())
        assigned_keyboards = (
            set().union(
                *(
                    platform_keyboards.get(platform_id, set())
                    for platform_id in assigned_platforms
                )
            )
            if assigned_platforms
            else set()
        )
        unrecorded = sorted(keyboard_ids - assigned_keyboards)
        if unrecorded:
            problems.append(
                f"{coverage_id} lists KEYBOARD_ID(s) absent from its platform row(s): "
                f"{', '.join(map(str, unrecorded))}"
            )
    for coverage_id in ("encoder-keyboard", "simultaneous-keyboards"):
        if len(coverage_platforms.get(coverage_id, set())) != 1:
            problems.append(f"{coverage_id} must identify exactly one platform ID")


def _validate_required_coverage(
    requirements: KeyboardRequirements,
    coverage_ids: dict[str, set[int]],
    coverage_platforms: dict[str, set[str]],
    platform_keyboards: dict[str, set[int]],
    problems: list[str],
) -> None:
    """Require all bundled, encoder, and simultaneous keyboard coverage."""

    for coverage_id in ("bundled-keyboards", "simultaneous-keyboards"):
        missing = sorted(
            requirements.bundled_ids - coverage_ids.get(coverage_id, set())
        )
        if missing:
            problems.append(
                f"{coverage_id} does not cover KEYBOARD_ID(s): "
                f"{', '.join(map(str, missing))}"
            )
    if not requirements.encoder_ids.intersection(
        coverage_ids.get("encoder-keyboard", set())
    ):
        problems.append("encoder-keyboard does not identify a bundled encoder keyboard")

    simultaneous_platforms = coverage_platforms.get("simultaneous-keyboards", set())
    if simultaneous_platforms:
        platform_id = next(iter(simultaneous_platforms))
        missing = sorted(
            requirements.bundled_ids - platform_keyboards.get(platform_id, set())
        )
        if missing:
            problems.append(
                "simultaneous-keyboards platform row does not list KEYBOARD_ID(s): "
                f"{', '.join(map(str, missing))}"
            )


def _validate_lifecycle_matrix(evidence: str, problems: list[str]) -> None:
    """Validate explicit upgrade, rollback, and uninstall results."""
    rows = _read_table(
        evidence,
        "Lifecycle results",
        ("Platform ID", "Upgrade", "Rollback", "Uninstall", "Evidence"),
        problems,
    )
    indexed = _index_rows(rows, EXPECTED_LIFECYCLE_IDS, "lifecycle", problems)
    for platform_id, row in indexed.items():
        for operation, result in zip(
            ("upgrade", "rollback", "uninstall"), row[1:4], strict=True
        ):
            if result != "PASS":
                problems.append(f"{platform_id} {operation} result must be PASS")
        if _is_placeholder(row[4]):
            problems.append(f"{platform_id} has no lifecycle evidence")


def _read_section(evidence: str, heading: str, problems: list[str]) -> str:
    """Read one exactly named Markdown section."""
    pattern = re.compile(
        rf"^### {re.escape(heading)}\s*$\n(.*?)(?=^#{{1,6}}\s|\Z)",
        re.MULTILINE | re.DOTALL,
    )
    sections = pattern.findall(evidence)
    if len(sections) != 1:
        problems.append(f"include exactly one {heading} section")
        return ""
    return sections[0]


def _read_table(
    evidence: str,
    heading: str,
    expected_header: tuple[str, ...],
    problems: list[str],
) -> list[list[str]]:
    """Read one strictly headed Markdown evidence table."""
    pattern = re.compile(
        rf"^### {re.escape(heading)}\s*$\n(.*?)(?=^#{{1,6}}\s|\Z)",
        re.MULTILINE | re.DOTALL,
    )
    sections = pattern.findall(evidence)
    if len(sections) != 1:
        problems.append(f"include exactly one {heading} section")
        return []
    table = [
        [cell.strip().strip("`") for cell in line.strip().strip("|").split("|")]
        for line in sections[0].splitlines()
        if line.strip().startswith("|") and line.strip().endswith("|")
    ]
    if len(table) < 2:
        problems.append(f"{heading} has no Markdown table")
        return []
    if tuple(table[0]) != expected_header:
        problems.append(f"{heading} has an invalid header")
        return []
    if len(table[1]) != len(expected_header) or not all(
        re.fullmatch(r":?-{3,}:?", cell) for cell in table[1]
    ):
        problems.append(f"{heading} has an invalid separator row")
        return []
    rows = table[2:]
    if any(len(row) != len(expected_header) for row in rows):
        problems.append(f"{heading} has a row with the wrong number of columns")
        return []
    return rows


def _index_rows(
    rows: list[list[str]],
    expected_ids: tuple[str, ...],
    label: str,
    problems: list[str],
) -> dict[str, list[str]]:
    """Index evidence rows while reporting missing, duplicate, and unknown IDs."""
    indexed: dict[str, list[str]] = {}
    duplicates: set[str] = set()
    unknown: set[str] = set()
    for row in rows:
        row_id = row[0]
        if row_id in indexed:
            duplicates.add(row_id)
        indexed[row_id] = row
        if row_id not in expected_ids:
            unknown.add(row_id)
    missing = sorted(set(expected_ids) - indexed.keys())
    if missing:
        problems.append(f"missing {label} rows: {', '.join(missing)}")
    if duplicates:
        problems.append(f"duplicate {label} rows: {', '.join(sorted(duplicates))}")
    if unknown:
        problems.append(f"unknown {label} rows: {', '.join(sorted(unknown))}")
    return {row_id: indexed[row_id] for row_id in expected_ids if row_id in indexed}


def _parse_keyboard_ids(value: str, label: str, problems: list[str]) -> set[int]:
    """Parse a comma-separated set of byte-sized keyboard IDs."""
    if not re.fullmatch(r"[0-9]+(?:\s*,\s*[0-9]+)*", value):
        problems.append(f"{label} has invalid KEYBOARD_ID(s)")
        return set()
    keyboard_ids = {int(item.strip()) for item in value.split(",")}
    if any(not 0 <= keyboard_id <= 255 for keyboard_id in keyboard_ids):
        problems.append(f"{label} has KEYBOARD_ID outside 0..255")
        return set()
    return keyboard_ids


def _parse_platform_ids(value: str, label: str, problems: list[str]) -> set[str]:
    """Parse a comma-separated set of stable platform IDs."""
    platform_ids = {item.strip() for item in value.split(",") if item.strip()}
    if not platform_ids or platform_ids - set(EXPECTED_PLATFORM_IDS):
        problems.append(f"{label} has invalid Platform ID(s)")
        return set()
    return platform_ids


def _is_placeholder(value: str) -> bool:
    """Return whether an evidence cell is empty or still a placeholder."""
    normalized = value.strip().strip("<>").lower()
    return normalized in PLACEHOLDER_VALUES or normalized.startswith("pending")


def _read_pull_request_event(path: Path) -> PullRequestEvent:
    """Read and validate a GitHub pull request event."""
    try:
        return PULL_REQUEST_EVENT_ADAPTER.validate_python(
            json.loads(path.read_text(encoding="utf-8"))
        )
    except (json.JSONDecodeError, ValidationError) as error:
        raise HardwareGateError(f"Invalid pull request event: {path}") from error


def _read_project_version(project: bytes) -> str:
    """Read the package version from pyproject.toml content."""
    try:
        version = tomllib.loads(project.decode())["project"]["version"]
    except (KeyError, TypeError, UnicodeDecodeError, tomllib.TOMLDecodeError) as error:
        raise HardwareGateError(
            "pyproject.toml has no valid project version"
        ) from error
    if not isinstance(version, str) or not version:
        raise HardwareGateError("pyproject.toml has no valid project version")
    return version


if __name__ == "__main__":
    app()

# Copyright 2026 sunaemon
# SPDX-License-Identifier: MIT
import logging
import re
from collections import defaultdict
from collections.abc import Iterable
from pathlib import Path
from typing import Annotated, Literal

import typer
from pydantic import (
    BaseModel,
    ConfigDict,
    Field,
    ValidationError,
    field_validator,
    model_validator,
)

from installer.release.check_hardware_gate import (
    EXPECTED_CHECK_IDS,
    EXPECTED_LIFECYCLE_IDS,
    EXPECTED_PLATFORM_IDS,
)
from model.src.util import initialize_logging

logger = logging.getLogger(__name__)

app = typer.Typer()

SHA_PATTERN = re.compile(r"[0-9a-fA-F]{40}")
AUTOMATION_CAPABLE_CHECK_IDS = frozenset(
    {
        "MAC-01",
        "MAC-02",
        "MAC-03",
        "MAC-04",
        "LX-02",
    }
)


class EvidenceCollectionError(Exception):
    """Raised when release evidence cannot be collected safely."""


class KeyboardIdentity(BaseModel):
    """Identify one physical keyboard participating in a test run."""

    model_config = ConfigDict(extra="forbid")

    name: str = Field(min_length=1)
    keyboard_id: int = Field(ge=0, le=255)
    firmware_revision: str = Field(min_length=1)


class CheckResult(BaseModel):
    """Record one explicit hardware-gate check result."""

    model_config = ConfigDict(extra="forbid")

    check_id: str
    result: Literal["PASS", "FAIL"]
    evidence_kind: Literal["automated", "physical", "manual"]
    detail: str = Field(min_length=1)

    @field_validator("check_id")
    @classmethod
    def validate_check_id(cls, value: str) -> str:
        """Reject results that cannot populate the release checklist."""
        if value not in EXPECTED_CHECK_IDS:
            raise ValueError(f"unknown hardware check ID: {value}")
        return value


class LifecycleResult(BaseModel):
    """Record explicit installer lifecycle results for one platform."""

    model_config = ConfigDict(extra="forbid")

    platform_id: str
    upgrade: Literal["PASS", "FAIL"]
    rollback: Literal["PASS", "FAIL"]
    uninstall: Literal["PASS", "FAIL"]

    @field_validator("platform_id")
    @classmethod
    def validate_platform_id(cls, value: str) -> str:
        """Reject lifecycle rows outside the stable gate schema."""
        if value not in EXPECTED_LIFECYCLE_IDS:
            raise ValueError(f"unknown lifecycle platform ID: {value}")
        return value


class EvidenceRecord(BaseModel):
    """Describe one exact-head automated or physical release test run."""

    model_config = ConfigDict(extra="forbid")

    schema_version: Literal[1]
    candidate_sha: str
    platform_id: str
    os_version: str = Field(min_length=1)
    session: str = Field(min_length=1)
    keyboards: list[KeyboardIdentity] = Field(min_length=1)
    transcript: Path
    checks: list[CheckResult] = Field(default_factory=list)
    lifecycle: LifecycleResult | None = None

    @model_validator(mode="after")
    def validate_result_scope(self) -> "EvidenceRecord":
        """Keep renderer and lifecycle results in their actual platform scope."""
        prefixes = {
            "macos-arm64-appkit": ("GLOBAL-", "MAC-"),
            "linux-x86_64-kde-wayland": ("GLOBAL-", "LX-", "KDE-"),
            "linux-x86_64-gnome-wayland": ("GLOBAL-", "LX-", "GNOME-"),
            "windows-x86_64-win32": ("GLOBAL-", "WIN-"),
        }
        if any(
            not check.check_id.startswith(prefixes[self.platform_id])
            for check in self.checks
        ):
            raise ValueError("Check does not belong to the recorded platform")
        lifecycle_platform = (
            "linux-x86_64"
            if self.platform_id.startswith("linux-")
            else self.platform_id
        )
        if (
            self.lifecycle is not None
            and self.lifecycle.platform_id != lifecycle_platform
        ):
            raise ValueError("Lifecycle does not belong to the recorded platform")
        return self

    @field_validator("candidate_sha")
    @classmethod
    def validate_candidate_sha(cls, value: str) -> str:
        """Require a complete immutable commit ID."""
        if SHA_PATTERN.fullmatch(value) is None:
            raise ValueError("candidate_sha must be a full 40-character SHA")
        return value.lower()

    @field_validator("platform_id")
    @classmethod
    def validate_platform_id(cls, value: str) -> str:
        """Require one stable platform matrix ID."""
        if value not in EXPECTED_PLATFORM_IDS:
            raise ValueError(f"unknown platform ID: {value}")
        return value


@app.command()
def main(
    candidate_sha: Annotated[
        str, typer.Option(help="Exact release candidate commit to summarize")
    ],
    records: Annotated[
        list[Path], typer.Option("--record", help="JSON evidence record to include")
    ],
    output: Annotated[
        Path | None, typer.Option(help="Write Markdown here instead of stdout")
    ] = None,
) -> None:
    """Generate an exact-head hardware evidence checklist summary."""
    initialize_logging()
    try:
        summary = collect_evidence(candidate_sha, records)
        if output is None:
            typer.echo(summary, nl=False)
        else:
            output.write_text(summary, encoding="utf-8")
            logger.info("Wrote hardware evidence summary to %s", output)
    except (EvidenceCollectionError, OSError):
        logger.exception("Failed to collect hardware release evidence")
        raise typer.Exit(code=1) from None


def collect_evidence(candidate_sha: str, record_paths: list[Path]) -> str:
    """Return a reviewable summary of current, stale, failed, and missing evidence."""
    if SHA_PATTERN.fullmatch(candidate_sha) is None:
        raise EvidenceCollectionError("Candidate must be a full 40-character SHA")
    candidate = candidate_sha.lower()
    records = [_read_record(path) for path in record_paths]
    return _render_summary(candidate, records)


def _read_record(path: Path) -> EvidenceRecord:
    """Read one validated evidence record and resolve its transcript path."""
    try:
        record = EvidenceRecord.model_validate_json(path.read_text(encoding="utf-8"))
    except (OSError, ValidationError) as error:
        raise EvidenceCollectionError(f"Invalid evidence record: {path}") from error
    transcript = record.transcript
    if not transcript.is_absolute():
        transcript = path.parent / transcript
    return record.model_copy(update={"transcript": transcript.resolve()})


def _render_summary(candidate: str, records: list[EvidenceRecord]) -> str:
    """Render evidence without promoting stale, failed, or transcript-less runs."""
    current = [record for record in records if record.candidate_sha == candidate]
    stale = [record for record in records if record.candidate_sha != candidate]
    checks: dict[str, list[tuple[EvidenceRecord, CheckResult]]] = defaultdict(list)
    lifecycle: dict[str, list[tuple[EvidenceRecord, LifecycleResult]]] = defaultdict(
        list
    )
    missing_transcripts: list[Path] = []
    for record in current:
        if not record.transcript.is_file():
            missing_transcripts.append(record.transcript)
            continue
        for result in record.checks:
            checks[result.check_id].append((record, result))
        if record.lifecycle is not None:
            lifecycle[record.lifecycle.platform_id].append((record, record.lifecycle))

    lines = [
        "# Hardware release evidence summary",
        "",
        f"Candidate commit: `{candidate}`",
        "",
        "## Checklist results",
        "",
        "| Check | Status | Kind | Source transcript | Detail |",
        "| ----- | ------ | ---- | ----------------- | ------ |",
    ]
    for check_id in EXPECTED_CHECK_IDS:
        lines.append(_check_row(check_id, checks.get(check_id, [])))

    lines.extend(
        [
            "",
            "## Lifecycle results",
            "",
            "| Platform ID | Upgrade | Rollback | Uninstall | Source transcript |",
            "| ----------- | ------- | -------- | --------- | ----------------- |",
        ]
    )
    for platform_id in EXPECTED_LIFECYCLE_IDS:
        lines.append(_lifecycle_row(platform_id, lifecycle.get(platform_id, [])))

    lines.extend(["", "## Run metadata", ""])
    if current:
        for record in current:
            keyboards = ", ".join(
                f"{keyboard.name} (ID {keyboard.keyboard_id}, {keyboard.firmware_revision})"
                for keyboard in record.keyboards
            )
            lines.append(
                f"- `{record.platform_id}` — {record.os_version}; {record.session}; "
                f"{keyboards}; `{record.transcript}`"
            )
    else:
        lines.append("- No records for this candidate.")

    lines.extend(["", "## Problems", ""])
    problems = _problem_lines(stale, missing_transcripts, checks, lifecycle)
    lines.extend(problems)
    lines.extend(
        [
            "- MANUAL REVIEW: reconcile all four platform rows and bundled, encoder, "
            "and simultaneous keyboard coverage in the release template. These are "
            "not inferred from check results.",
            "- MANUAL REVIEW: only GLOBAL-01/02 may use a reasoned N/A in the PR "
            "when the release delta permits it; the gate validates eligibility.",
            "- This summary is a review aid, not a hardware-gate pass. Preserve "
            "transcripts and wait for hardware-release-gate on the release PR.",
        ]
    )
    lines.append("")
    return "\n".join(lines)


def _check_row(check_id: str, entries: list[tuple[EvidenceRecord, CheckResult]]) -> str:
    """Render one checklist result, keeping conflicts visibly blocking."""
    if not entries:
        return f"| {check_id} | MISSING | - | - | - |"
    statuses = {result.result for _, result in entries}
    has_tester_input = any(
        result.evidence_kind in {"manual", "physical"} for _, result in entries
    )
    if "FAIL" in statuses:
        status = "FAIL"
    elif check_id not in AUTOMATION_CAPABLE_CHECK_IDS and not has_tester_input:
        status = "INCOMPLETE"
    else:
        status = "PASS"
    kinds = ", ".join(sorted({result.evidence_kind for _, result in entries}))
    sources = "<br>".join(f"`{record.transcript}`" for record, _ in entries)
    details = "<br>".join(_escape_cell(result.detail) for _, result in entries)
    return f"| {check_id} | {status} | {kinds} | {sources} | {details} |"


def _lifecycle_row(
    platform_id: str, entries: list[tuple[EvidenceRecord, LifecycleResult]]
) -> str:
    """Render one lifecycle row, requiring all supplied runs to pass."""
    if not entries:
        return f"| {platform_id} | MISSING | MISSING | MISSING | - |"
    operations = (
        _combined_result(result.upgrade for _, result in entries),
        _combined_result(result.rollback for _, result in entries),
        _combined_result(result.uninstall for _, result in entries),
    )
    sources = "<br>".join(f"`{record.transcript}`" for record, _ in entries)
    return f"| {platform_id} | {' | '.join(operations)} | {sources} |"


def _combined_result(results: Iterable[str]) -> str:
    """Return PASS only when every recorded result passes."""
    return "PASS" if set(results) == {"PASS"} else "FAIL"


def _problem_lines(
    stale: list[EvidenceRecord],
    missing_transcripts: list[Path],
    checks: dict[str, list[tuple[EvidenceRecord, CheckResult]]],
    lifecycle: dict[str, list[tuple[EvidenceRecord, LifecycleResult]]],
) -> list[str]:
    """List every condition that prevents a complete exact-head draft."""
    problems = [
        f"- STALE: `{record.transcript}` belongs to `{record.candidate_sha}`."
        for record in stale
    ]
    problems.extend(
        f"- INCOMPLETE: source transcript does not exist: `{path}`."
        for path in missing_transcripts
    )
    failed = sorted(
        check_id
        for check_id, entries in checks.items()
        if any(result.result == "FAIL" for _, result in entries)
    )
    if failed:
        problems.append(f"- FAILED checks: {', '.join(failed)}.")
    for platform_id, entries in lifecycle.items():
        if any(
            "FAIL" in (result.upgrade, result.rollback, result.uninstall)
            for _, result in entries
        ):
            problems.append(f"- FAILED lifecycle: {platform_id}.")
    automated_only = sorted(
        check_id
        for check_id, entries in checks.items()
        if check_id not in AUTOMATION_CAPABLE_CHECK_IDS
        and not any(
            result.evidence_kind in {"manual", "physical"} for _, result in entries
        )
    )
    if automated_only:
        problems.append(
            "- INCOMPLETE tester inputs (automated evidence is insufficient): "
            f"{', '.join(automated_only)}."
        )
    missing = sorted(set(EXPECTED_CHECK_IDS) - checks.keys())
    if missing:
        problems.append(f"- MISSING checks: {', '.join(missing)}.")
    missing_lifecycle = sorted(set(EXPECTED_LIFECYCLE_IDS) - lifecycle.keys())
    if missing_lifecycle:
        problems.append(f"- MISSING lifecycle results: {', '.join(missing_lifecycle)}.")
    return problems


def _escape_cell(value: str) -> str:
    """Keep record detail inside one Markdown table cell."""
    return value.replace("|", "\\|").replace("\n", "<br>")


if __name__ == "__main__":
    app()

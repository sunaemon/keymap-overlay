# Copyright 2026 sunaemon
# SPDX-License-Identifier: MIT
import logging
import re
from pathlib import Path
from typing import Annotated

import typer
from pydantic import ValidationError

from installer.release.collect_hardware_evidence import (
    CheckResult,
    EvidenceRecord,
    KeyboardIdentity,
    LifecycleResult,
)
from model.src.util import initialize_logging

logger = logging.getLogger(__name__)

app = typer.Typer()

PROFILE_RESULTS = {
    "macos-session": (
        "PASS: macOS live startup, Vial reread, labels, layer transitions, focus,",
        ("MAC-01", "MAC-02", "MAC-03", "MAC-04"),
        "automated",
    ),
    "macos-physical-reports": (
        "PASS: every configured physical MO key emitted ordered press/release Raw HID reports",
        ("MAC-08",),
        "physical",
    ),
    "macos-login": (
        "PASS: actual sign-out/sign-in startup and interactive HIL layer event",
        ("MAC-10",),
        "physical",
    ),
    "linux-kde-session": (
        "PASS: installed Linux virtual Vial device, ten Raw HID cycles, ordering, D-Bus state, Qt accessibility labels, and focus retention",
        ("LX-02", "LX-03", "KDE-01"),
        "automated",
    ),
    "linux-physical-reports": (
        "PASS: every configured physical MO key emitted ordered press/release Raw HID reports",
        ("LX-08",),
        "physical",
    ),
}


class EvidenceRecordError(Exception):
    """Raised when command-line evidence fields are invalid."""


@app.command()
def main(
    output: Annotated[Path, typer.Option(help="JSON record to create")],
    candidate_sha: Annotated[str, typer.Option(help="Exact tested commit")],
    platform_id: Annotated[str, typer.Option(help="Stable release platform ID")],
    os_version: Annotated[str, typer.Option(help="Tested operating system version")],
    session: Annotated[str, typer.Option(help="Tested desktop, session, and renderer")],
    transcript: Annotated[Path, typer.Option(help="Saved source transcript")],
    keyboards: Annotated[
        list[str],
        typer.Option(
            "--keyboard",
            help="Keyboard metadata as NAME|KEYBOARD_ID|FIRMWARE_REVISION",
        ),
    ],
    checks: Annotated[
        list[str],
        typer.Option("--check", help="Result as CHECK_ID|PASS_OR_FAIL|KIND|DETAIL"),
    ] = [],
    lifecycle: Annotated[
        str | None,
        typer.Option(help="Lifecycle as PLATFORM_ID|UPGRADE|ROLLBACK|UNINSTALL"),
    ] = None,
    profile: Annotated[
        str | None,
        typer.Option(help="Recognized HIL transcript profile whose results to import"),
    ] = None,
) -> None:
    """Write one validated, transcript-backed hardware evidence record."""
    initialize_logging()
    try:
        record = build_record(
            candidate_sha=candidate_sha,
            platform_id=platform_id,
            os_version=os_version,
            session=session,
            transcript=transcript,
            keyboards=keyboards,
            checks=checks,
            lifecycle=lifecycle,
            profile=profile,
        )
        output.write_text(
            record.model_dump_json(indent=2, exclude_none=True) + "\n",
            encoding="utf-8",
        )
        logger.info("Wrote hardware evidence record to %s", output)
    except (EvidenceRecordError, OSError, ValidationError):
        logger.exception("Failed to record hardware release evidence")
        raise typer.Exit(code=1) from None


def build_record(
    *,
    candidate_sha: str,
    platform_id: str,
    os_version: str,
    session: str,
    transcript: Path,
    keyboards: list[str],
    checks: list[str],
    lifecycle: str | None,
    profile: str | None = None,
) -> EvidenceRecord:
    """Build one record from compact repeatable command-line fields."""
    if not transcript.is_file():
        raise EvidenceRecordError(f"Transcript does not exist: {transcript}")
    parsed_checks = [_parse_check(value) for value in checks]
    if profile is not None:
        parsed_checks.extend(_read_profile_results(profile, transcript, candidate_sha))
    return EvidenceRecord(
        schema_version=1,
        candidate_sha=candidate_sha,
        platform_id=platform_id,
        os_version=os_version,
        session=session,
        transcript=transcript.resolve(),
        keyboards=[_parse_keyboard(value) for value in keyboards],
        checks=parsed_checks,
        lifecycle=_parse_lifecycle(lifecycle) if lifecycle is not None else None,
    )


def _read_profile_results(
    profile: str, transcript: Path, candidate_sha: str
) -> list[CheckResult]:
    """Import only documented results from a successful exact-head HIL transcript."""
    try:
        marker, check_ids, evidence_kind = PROFILE_RESULTS[profile]
    except KeyError as error:
        profiles = ", ".join(sorted(PROFILE_RESULTS))
        raise EvidenceRecordError(
            f"Unknown transcript profile {profile}; expected one of: {profiles}"
        ) from error
    content = transcript.read_text(encoding="utf-8")
    candidates = re.findall(r"^Candidate: ([0-9a-fA-F]{40})$", content, re.MULTILINE)
    if [candidate.lower() for candidate in candidates] != [candidate_sha.lower()]:
        raise EvidenceRecordError(
            "Transcript must contain exactly one Candidate line matching --candidate-sha"
        )
    if marker not in content:
        raise EvidenceRecordError(
            f"Transcript does not contain the {profile} success marker"
        )
    return [
        CheckResult.model_validate(
            {
                "check_id": check_id,
                "result": "PASS",
                "evidence_kind": evidence_kind,
                "detail": f"Imported from {profile} success marker",
            }
        )
        for check_id in check_ids
    ]


def _parse_keyboard(value: str) -> KeyboardIdentity:
    """Parse one reusable keyboard identity argument."""
    fields = value.split("|", maxsplit=2)
    if len(fields) != 3:
        raise EvidenceRecordError("Keyboard must be NAME|KEYBOARD_ID|FIRMWARE_REVISION")
    try:
        keyboard_id = int(fields[1])
    except ValueError as error:
        raise EvidenceRecordError(f"Invalid KEYBOARD_ID: {fields[1]}") from error
    return KeyboardIdentity(
        name=fields[0], keyboard_id=keyboard_id, firmware_revision=fields[2]
    )


def _parse_check(value: str) -> CheckResult:
    """Parse one explicit result without inferring physical observations."""
    fields = value.split("|", maxsplit=3)
    if len(fields) != 4:
        raise EvidenceRecordError("Check must be CHECK_ID|PASS_OR_FAIL|KIND|DETAIL")
    return CheckResult.model_validate(
        {
            "check_id": fields[0],
            "result": fields[1],
            "evidence_kind": fields[2],
            "detail": fields[3],
        }
    )


def _parse_lifecycle(value: str) -> LifecycleResult:
    """Parse one explicit lifecycle result set."""
    fields = value.split("|")
    if len(fields) != 4:
        raise EvidenceRecordError(
            "Lifecycle must be PLATFORM_ID|UPGRADE|ROLLBACK|UNINSTALL"
        )
    return LifecycleResult.model_validate(
        {
            "platform_id": fields[0],
            "upgrade": fields[1],
            "rollback": fields[2],
            "uninstall": fields[3],
        }
    )


if __name__ == "__main__":
    app()

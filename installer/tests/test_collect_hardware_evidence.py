# Copyright 2026 sunaemon
# SPDX-License-Identifier: MIT
import json
from pathlib import Path

import pytest

from installer.release.collect_hardware_evidence import (
    EvidenceCollectionError,
    collect_evidence,
)

CANDIDATE = "1" * 40
STALE_CANDIDATE = "2" * 40


def test_collects_passing_results_with_metadata_and_transcript(tmp_path: Path) -> None:
    """A current passing record populates results and reusable run metadata."""
    record = write_record(
        tmp_path,
        candidate=CANDIDATE,
        checks=[check("MAC-01", "PASS", "automated")],
        lifecycle={
            "platform_id": "macos-arm64-appkit",
            "upgrade": "PASS",
            "rollback": "PASS",
            "uninstall": "PASS",
        },
    )

    summary = collect_evidence(CANDIDATE, [record])

    assert "| MAC-01 | PASS | automated |" in summary
    assert "| macos-arm64-appkit | PASS | PASS | PASS |" in summary
    assert "Insixty (ID 1, deadbeef)" in summary
    assert str(tmp_path / "run.log") in summary


def test_failed_result_remains_failed(tmp_path: Path) -> None:
    """A failed run is reported as blocking even beside a passing run."""
    passing = write_record(
        tmp_path,
        name="passing.json",
        candidate=CANDIDATE,
        checks=[check("MAC-01", "PASS", "automated")],
    )
    failed = write_record(
        tmp_path,
        name="failed.json",
        transcript="failed.log",
        candidate=CANDIDATE,
        checks=[check("MAC-01", "FAIL", "manual")],
    )

    summary = collect_evidence(CANDIDATE, [passing, failed])

    assert "| MAC-01 | FAIL | automated, manual |" in summary
    assert "- FAILED checks: MAC-01." in summary


def test_missing_transcript_does_not_populate_result(tmp_path: Path) -> None:
    """A result without its claimed source transcript stays incomplete."""
    record = write_record(
        tmp_path,
        candidate=CANDIDATE,
        checks=[check("MAC-01", "PASS", "automated")],
        create_transcript=False,
    )

    summary = collect_evidence(CANDIDATE, [record])

    assert "| MAC-01 | MISSING | - | - | - |" in summary
    assert "INCOMPLETE: source transcript does not exist" in summary


def test_stale_record_is_flagged_and_does_not_populate_result(tmp_path: Path) -> None:
    """Evidence from another commit is visibly stale and never promoted."""
    record = write_record(
        tmp_path,
        candidate=STALE_CANDIDATE,
        checks=[check("MAC-01", "PASS", "automated")],
    )

    summary = collect_evidence(CANDIDATE, [record])

    assert "| MAC-01 | MISSING | - | - | - |" in summary
    assert f"STALE: `{tmp_path / 'run.log'}` belongs to `{STALE_CANDIDATE}`" in summary


def test_rejects_unknown_check_id(tmp_path: Path) -> None:
    """A record cannot invent a checklist result outside the gate schema."""
    record = write_record(
        tmp_path,
        candidate=CANDIDATE,
        checks=[check("MAC-99", "PASS", "automated")],
    )

    with pytest.raises(EvidenceCollectionError, match="Invalid evidence record"):
        collect_evidence(CANDIDATE, [record])


def test_automated_result_cannot_satisfy_human_only_check(tmp_path: Path) -> None:
    """Automation never infers a visual or physical tester observation."""
    record = write_record(
        tmp_path,
        candidate=CANDIDATE,
        checks=[check("MAC-05", "PASS", "automated")],
    )

    summary = collect_evidence(CANDIDATE, [record])

    assert "| MAC-05 | INCOMPLETE | automated |" in summary
    assert (
        "INCOMPLETE tester inputs (automated evidence is insufficient): MAC-05"
        in summary
    )


def check(check_id: str, result: str, kind: str) -> dict[str, str]:
    """Return one evidence check object."""
    return {
        "check_id": check_id,
        "result": result,
        "evidence_kind": kind,
        "detail": "observed result",
    }


def write_record(
    tmp_path: Path,
    *,
    candidate: str,
    checks: list[dict[str, str]],
    name: str = "record.json",
    transcript: str = "run.log",
    lifecycle: dict[str, str] | None = None,
    create_transcript: bool = True,
) -> Path:
    """Write one representative evidence record and optional transcript."""
    if create_transcript:
        (tmp_path / transcript).write_text("test transcript\n")
    record = tmp_path / name
    record.write_text(
        json.dumps(
            {
                "schema_version": 1,
                "candidate_sha": candidate,
                "platform_id": "macos-arm64-appkit",
                "os_version": "macOS 26.0",
                "session": "AppKit / Aqua",
                "keyboards": [
                    {
                        "name": "Insixty",
                        "keyboard_id": 1,
                        "firmware_revision": "deadbeef",
                    }
                ],
                "transcript": transcript,
                "checks": checks,
                "lifecycle": lifecycle,
            }
        )
    )
    return record

# Copyright 2026 sunaemon
# SPDX-License-Identifier: MIT
from pathlib import Path

import pytest

from installer.release.record_hardware_evidence import (
    EvidenceRecordError,
    build_record,
)

CANDIDATE = "a" * 40


def test_builds_validated_record_from_compact_fields(tmp_path: Path) -> None:
    """Shared run metadata and explicit results become one reusable record."""
    transcript = tmp_path / "run.log"
    transcript.write_text("PASS\n")

    record = build_record(
        candidate_sha=CANDIDATE,
        platform_id="macos-arm64-appkit",
        os_version="macOS 26.0",
        session="AppKit / Aqua",
        transcript=transcript,
        keyboards=["Insixty|1|firmware-abc"],
        checks=["MAC-01|PASS|automated|startup and model read passed"],
        lifecycle="macos-arm64-appkit|PASS|PASS|PASS",
        profile=None,
    )

    assert record.candidate_sha == CANDIDATE
    assert record.keyboards[0].keyboard_id == 1
    assert record.checks[0].evidence_kind == "automated"
    assert record.lifecycle is not None
    assert record.lifecycle.uninstall == "PASS"


def test_requires_source_transcript(tmp_path: Path) -> None:
    """A record cannot be created for an absent transcript."""
    with pytest.raises(EvidenceRecordError, match="Transcript does not exist"):
        build_record(
            candidate_sha=CANDIDATE,
            platform_id="macos-arm64-appkit",
            os_version="macOS 26.0",
            session="AppKit / Aqua",
            transcript=tmp_path / "missing.log",
            keyboards=["Insixty|1|firmware-abc"],
            checks=[],
            lifecycle=None,
        )


def test_imports_supported_results_from_exact_head_hil_transcript(
    tmp_path: Path,
) -> None:
    """Known HIL output populates results without retyping them."""
    transcript = tmp_path / "macos-session.log"
    transcript.write_text(
        f"Candidate: {CANDIDATE}\n"
        "PASS: macOS live startup, Vial reread, labels, layer transitions, focus, "
        "click-through, topmost\n"
    )

    record = build_record(
        candidate_sha=CANDIDATE,
        platform_id="macos-arm64-appkit",
        os_version="macOS 26.0",
        session="AppKit / Aqua",
        transcript=transcript,
        keyboards=["Insixty|1|firmware-abc"],
        checks=[],
        lifecycle=None,
        profile="macos-session",
    )

    assert [result.check_id for result in record.checks] == [
        "MAC-01",
        "MAC-02",
        "MAC-03",
        "MAC-04",
    ]


def test_profile_rejects_stale_transcript(tmp_path: Path) -> None:
    """A recognized success marker cannot be imported from another commit."""
    transcript = tmp_path / "macos-session.log"
    transcript.write_text(
        f"Candidate: {'b' * 40}\n"
        "PASS: macOS live startup, Vial reread, labels, layer transitions, focus,\n"
    )

    with pytest.raises(EvidenceRecordError, match="matching --candidate-sha"):
        build_record(
            candidate_sha=CANDIDATE,
            platform_id="macos-arm64-appkit",
            os_version="macOS 26.0",
            session="AppKit / Aqua",
            transcript=transcript,
            keyboards=["Insixty|1|firmware-abc"],
            checks=[],
            lifecycle=None,
            profile="macos-session",
        )

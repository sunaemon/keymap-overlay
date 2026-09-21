# Copyright 2026 sunaemon
# SPDX-License-Identifier: MIT
import shutil
from pathlib import Path

import pytest
from typer.testing import CliRunner

from installer.release.collect_hardware_evidence import collect_evidence
from installer.release.record_hardware_evidence import (
    EvidenceRecordError,
    app,
    build_record,
    write_record_bundle,
)

CANDIDATE = "a" * 40


def test_cli_creates_portable_manual_bundle(tmp_path: Path) -> None:
    """The documented repeated metadata arguments produce transferable files."""
    transcript = tmp_path / "observations.txt"
    transcript.write_text("Tester observed normal typing on all shows")
    output = tmp_path / "windows" / "window.json"
    result = CliRunner().invoke(
        app,
        [
            "--candidate-sha",
            CANDIDATE,
            "--platform-id",
            "windows-x86_64-win32",
            "--os-version",
            "Windows 11",
            "--session",
            "Win32 / desktop",
            "--keyboard",
            "Insixty|1|abc",
            "--transcript",
            str(transcript),
            "--check",
            "WIN-04|PASS|physical|Typing reached editor on repeated shows",
            "--output",
            str(output),
        ],
    )
    assert result.exit_code == 0, result.output
    assert "| WIN-04 | PASS | physical |" in collect_evidence(CANDIDATE, [output])


def test_bundle_survives_transfer_to_another_machine(tmp_path: Path) -> None:
    """A copied bundle resolves its transcript without the source machine path."""
    source = tmp_path / "source.txt"
    source.write_text("Tester observed correct labels")
    record = build_record(
        candidate_sha=CANDIDATE,
        platform_id="macos-arm64-appkit",
        os_version="macOS 26",
        session="Aqua",
        transcript=source,
        keyboards=["Insixty|1|abc"],
        checks=["MAC-05|PASS|manual|Compared with live Vial"],
        lifecycle=None,
    )
    output = tmp_path / "tester" / "visual.json"
    write_record_bundle(record, output)
    shutil.copytree(output.parent, tmp_path / "reviewer")
    source.unlink()
    summary = collect_evidence(CANDIDATE, [tmp_path / "reviewer" / "visual.json"])
    assert "| MAC-05 | PASS | manual |" in summary
    assert str(tmp_path / "reviewer" / "visual.log") in summary
    with pytest.raises(EvidenceRecordError, match="already exists"):
        write_record_bundle(record, output)


def test_kde_profile_does_not_claim_vial_edit_or_physical_startup(
    tmp_path: Path,
) -> None:
    """Virtual fixture output supplies only the complete layer-order check."""
    transcript = tmp_path / "linux.log"
    transcript.write_text(
        f"Candidate: {CANDIDATE}\n"
        "PASS: installed Linux virtual Vial device, ten Raw HID cycles, ordering, D-Bus state, Qt accessibility labels, and focus retention\n"
    )
    record = build_record(
        candidate_sha=CANDIDATE,
        platform_id="linux-x86_64-kde-wayland",
        os_version="Fedora",
        session="KDE / Wayland",
        transcript=transcript,
        keyboards=["Insixty|1|abc"],
        checks=[],
        lifecycle=None,
        profile="linux-kde-session",
    )
    assert [check.check_id for check in record.checks] == ["LX-02"]


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

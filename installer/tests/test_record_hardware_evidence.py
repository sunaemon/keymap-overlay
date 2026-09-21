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


@pytest.mark.parametrize("suffix", [".json", ".JSON"])
def test_bundle_survives_transfer_to_another_machine(
    tmp_path: Path, suffix: str
) -> None:
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
    output = tmp_path / "tester" / f"visual{suffix}"
    write_record_bundle(record, output)
    shutil.copytree(output.parent, tmp_path / "reviewer")
    source.unlink()
    summary = collect_evidence(CANDIDATE, [tmp_path / "reviewer" / output.name])
    assert "| MAC-05 | PASS | manual |" in summary
    assert str(tmp_path / "reviewer" / "visual.log") in summary
    with pytest.raises(EvidenceRecordError, match="already exists"):
        write_record_bundle(record, output)


@pytest.mark.parametrize(
    "filename", ["evidence.log", "evidence.LOG", "evidence.txt", "evidence"]
)
def test_invalid_output_suffix_preserves_transcript(
    tmp_path: Path, filename: str
) -> None:
    """Reject ambiguous bundle paths before copying or overwriting evidence."""
    source = tmp_path / "source.log"
    source.write_text("Original physical observation\n")
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
    output = tmp_path / "bundle" / filename
    with pytest.raises(EvidenceRecordError, match="must use a .json suffix"):
        write_record_bundle(record, output)
    assert source.read_text() == "Original physical observation\n"
    assert not output.parent.exists()


def test_kde_profile_imports_restart_and_transition_checks(
    tmp_path: Path,
) -> None:
    """The Linux HIL supplies restart-read and deterministic transition checks."""
    transcript = tmp_path / "linux.log"
    transcript.write_text(
        f"Candidate: {CANDIDATE}\n"
        "PASS: Linux live Vial restart read, installed virtual Vial device, ten Raw HID cycles, ordering, D-Bus state, Qt accessibility labels, and focus retention\n"
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
    assert [check.check_id for check in record.checks] == [
        "LX-02",
        "LX-03",
        "LX-08",
    ]


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


def test_imports_platform_independent_physical_mo_result(tmp_path: Path) -> None:
    """A guided physical transcript populates the release-wide switch proof."""
    transcript = tmp_path / "physical-reports.log"
    transcript.write_text(
        f"Candidate: {CANDIDATE}\n"
        "PASS: every configured physical MO key emitted ordered press/release Raw HID reports\n"
    )

    record = build_record(
        candidate_sha=CANDIDATE,
        platform_id="linux-x86_64-gnome-wayland",
        os_version="Arch Linux",
        session="GNOME / Wayland",
        transcript=transcript,
        keyboards=["Insixty|1|firmware-abc", "DOIO KB16|2|firmware-abc"],
        checks=[],
        lifecycle=None,
        profile="physical-mo-reports",
    )

    assert record.checks[0].check_id == "GLOBAL-03"
    assert record.checks[0].evidence_kind == "physical"


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

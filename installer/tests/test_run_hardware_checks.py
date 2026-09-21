# Copyright 2026 sunaemon
# SPDX-License-Identifier: MIT
import re
import runpy
import subprocess
import sys
from pathlib import Path

import pytest
from typer.testing import CliRunner

from installer.release import run_hardware_checks as runner
from installer.release.record_hardware_evidence import build_record

CANDIDATE = "a" * 40


@pytest.mark.parametrize(
    ("system", "machine", "desktop", "session", "expected"),
    [
        ("Darwin", "arm64", "", "", "macos-arm64-appkit"),
        ("Windows", "AMD64", "", "", "windows-x86_64-win32"),
        ("Linux", "x86_64", "KDE", "wayland", "linux-x86_64-kde-wayland"),
        ("Linux", "x86_64", "ubuntu:GNOME", "wayland", "linux-x86_64-gnome-wayland"),
    ],
)
def test_detects_supported_sessions(
    monkeypatch: pytest.MonkeyPatch,
    system: str,
    machine: str,
    desktop: str,
    session: str,
    expected: str,
) -> None:
    """Native host/session metadata selects only its own renderer workflow."""
    monkeypatch.setattr(runner.platform, "system", lambda: system)
    monkeypatch.setattr(runner.platform, "machine", lambda: machine)
    monkeypatch.setenv("XDG_CURRENT_DESKTOP", desktop)
    monkeypatch.setenv("XDG_SESSION_TYPE", session)
    assert runner.detect_platform()[0] == expected


@pytest.mark.parametrize(("desktop", "session"), [("KDE", "x11"), ("sway", "wayland")])
def test_rejects_unsupported_session(
    monkeypatch: pytest.MonkeyPatch, desktop: str, session: str
) -> None:
    """Other desktops and X11 are not attributed to a required renderer row."""
    monkeypatch.setattr(runner.platform, "system", lambda: "Linux")
    monkeypatch.setattr(runner.platform, "machine", lambda: "x86_64")
    monkeypatch.setenv("XDG_CURRENT_DESKTOP", desktop)
    monkeypatch.setenv("XDG_SESSION_TYPE", session)
    with pytest.raises(ValueError, match="Unsupported"):
        runner.detect_platform()


def test_cleanup_failure_overrides_success_marker(tmp_path: Path) -> None:
    """A helper's early PASS does not survive its nonzero final exit status."""
    transcript = tmp_path / "session.log"
    transcript.write_text(
        f"Candidate: {CANDIDATE}\n"
        "PASS: macOS live startup, Vial reread, labels, layer transitions, focus,\n"
        "ERROR: restore failed\nExit status: 1\n"
    )
    base = build_record(
        candidate_sha=CANDIDATE,
        platform_id="macos-arm64-appkit",
        os_version="macOS",
        session="Aqua",
        transcript=transcript,
        keyboards=["Insixty|1|abc"],
        checks=[],
        lifecycle=None,
    )
    record = runner.helper_record(base, transcript, "macos-session", 1)
    assert len(record.checks) == 4
    assert {check.result for check in record.checks} == {"FAIL"}
    assert runner.helper_record(base, transcript, None, 0).checks == []


def test_plan_does_not_execute_or_write(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """Previewing a plan is hardware-free and creates no evidence."""
    setup_host(tmp_path, monkeypatch)
    output = tmp_path / "evidence"
    result = CliRunner().invoke(runner.app, args(output) + ["--plan"])
    assert result.exit_code == 0, result.output
    assert not output.exists()


def test_guided_manual_results_and_skips(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """Only explicit observations become results; skips remain missing."""
    setup_host(tmp_path, monkeypatch)
    output = tmp_path / "evidence"
    result = CliRunner().invoke(
        runner.app,
        args(output),
        input="y\nPASS\nObserved typing across repeated shows\nSKIP\n",
    )
    assert result.exit_code == 0, result.output
    assert (output / "WIN-04.json").exists()
    summary = (output / "summary.md").read_text()
    assert "| WIN-04 | PASS | manual |" in summary
    assert "| WIN-05 | MISSING |" in summary


def test_failed_observation_stops_and_preserves_results(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """A tester failure saves its evidence and prevents a successful exit."""
    setup_host(tmp_path, monkeypatch)
    output = tmp_path / "evidence"
    result = CliRunner().invoke(
        runner.app, args(output), input="y\nFAIL\nFocus moved to overlay\n"
    )
    assert result.exit_code == 1
    summary = (output / "summary.md").read_text()
    assert "STOPPED before completion" in summary
    assert "| WIN-04 | FAIL | manual |" in summary


def test_candidate_change_stops_recording(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """Changing the checkout cannot produce a PASS for the earlier candidate."""
    setup_host(tmp_path, monkeypatch)
    candidates = iter([CANDIDATE, "b" * 40])
    monkeypatch.setattr(runner, "clean_candidate", lambda: next(candidates))
    output = tmp_path / "evidence"
    result = CliRunner().invoke(
        runner.app, args(output), input="y\nPASS\nObserved typing\n"
    )
    assert result.exit_code == 1
    assert not (output / "WIN-04.json").exists()
    assert "STOPPED" in (output / "summary.md").read_text()


def test_checklist_comes_from_release_template() -> None:
    """The guided procedure covers the current shared and renderer IDs."""
    root = Path(__file__).resolve().parents[2]
    for platform_id in runner.HELPERS:
        descriptions = runner.check_descriptions(root, platform_id)
        assert len(descriptions) == 10
        assert all(descriptions.values())


def test_guided_hil_imports_only_completed_checks(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """The guided flow retains physical logs while importing only session assertions."""
    setup_host(tmp_path, monkeypatch)
    monkeypatch.setattr(
        runner, "detect_platform", lambda: ("macos-arm64-appkit", "Aqua")
    )
    monkeypatch.setattr(
        runner,
        "check_descriptions",
        lambda *args: {"MAC-01": "startup", "MAC-08": "physical plus repeated cycles"},
    )

    def fake_helper(target: str, root: Path, transcript: Path) -> int:
        transcript.write_text(
            f"Candidate: {CANDIDATE}\n"
            "PASS: macOS live startup, Vial reread, labels, layer transitions, focus,\n"
        )
        return 0

    monkeypatch.setattr(runner, "run_helper", fake_helper)
    output = tmp_path / "evidence"
    result = CliRunner().invoke(runner.app, args(output), input="y\nSKIP\n")
    assert result.exit_code == 0, result.output
    summary = (output / "summary.md").read_text()
    assert "| MAC-01 | PASS | automated |" in summary
    assert "| MAC-08 | MISSING |" in summary
    assert (output / "test-hardware-physical-reports-macos.log").exists()


def test_helper_capture_includes_output_and_exit(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """Command capture preserves stderr and a nonzero final status without hardware."""
    popen = subprocess.Popen

    def launch(
        command: list[str],
        *,
        cwd: Path,
        env: dict[str, str],
        stdout: int,
        stderr: int,
        text: bool,
        encoding: str,
        errors: str,
    ) -> subprocess.Popen[str]:
        assert command == ["make", "test-hardware-session-macos"]
        return popen(
            [
                sys.executable,
                "-c",
                "import sys; print('helper output'); print('failure', file=sys.stderr); sys.exit(3)",
            ],
            cwd=cwd,
            env=env,
            stdout=stdout,
            stderr=stderr,
            text=text,
            encoding=encoding,
            errors=errors,
        )

    monkeypatch.setattr(runner.subprocess, "Popen", launch)
    transcript = tmp_path / "helper.txt"
    assert runner.run_helper("test-hardware-session-macos", tmp_path, transcript) == 3
    content = transcript.read_text()
    assert "helper output" in content
    assert "failure" in content
    assert "Exit status: 3" in content


def test_failed_helper_stops_before_observation_prompts(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """A failed physical helper retains evidence and prevents later HIL execution."""
    setup_host(tmp_path, monkeypatch)
    monkeypatch.setattr(
        runner, "detect_platform", lambda: ("macos-arm64-appkit", "Aqua")
    )
    calls = []

    def failed(target: str, root: Path, transcript: Path) -> int:
        calls.append(target)
        transcript.write_text("Physical report timed out\nExit status: 1\n")
        return 1

    monkeypatch.setattr(runner, "run_helper", failed)
    output = tmp_path / "evidence"
    result = CliRunner().invoke(runner.app, args(output), input="y\n")
    assert result.exit_code == 1
    assert calls == ["test-hardware-physical-reports-macos"]
    assert "STOPPED" in (output / "summary.md").read_text()
    assert (
        "timed out" in (output / "test-hardware-physical-reports-macos.log").read_text()
    )


@pytest.mark.parametrize(
    "invalid", ["inside-checkout", "existing-directory", "blank-tester"]
)
def test_invalid_run_configuration_preserves_files(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, invalid: str
) -> None:
    """Invalid runs stop before prompting or changing candidate/previous evidence."""
    setup_host(tmp_path, monkeypatch)
    output = tmp_path / "evidence"
    if invalid == "inside-checkout":
        output = tmp_path / "checkout" / "evidence"
    if invalid == "existing-directory":
        output.mkdir()
        (output / "previous.txt").write_text("Keep previous evidence")
    arguments = args(output)
    if invalid == "blank-tester":
        arguments[arguments.index("--tester") + 1] = "   "
    before = sorted(tmp_path.rglob("*"))
    result = CliRunner().invoke(runner.app, arguments)
    assert result.exit_code == 1
    assert "Proceed?" not in result.output
    assert sorted(tmp_path.rglob("*")) == before
    if invalid == "existing-directory":
        assert (output / "previous.txt").read_text() == "Keep previous evidence"


def test_blank_observation_cannot_become_a_pass(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """Whitespace does not satisfy the requirement for an explicit observation."""
    setup_host(tmp_path, monkeypatch)
    output = tmp_path / "evidence"
    result = CliRunner().invoke(runner.app, args(output), input="y\nPASS\n   \n")
    assert result.exit_code == 1
    assert not (output / "WIN-04.json").exists()
    summary = (output / "summary.md").read_text()
    assert "STOPPED" in summary
    assert "| WIN-04 | MISSING |" in summary


def test_invalid_outcome_reprompts_before_recording(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """An unsupported answer cannot be recorded, and corrected lowercase input works."""
    setup_host(tmp_path, monkeypatch)
    output = tmp_path / "evidence"
    result = CliRunner().invoke(
        runner.app, args(output), input="y\nsure\npass\nObserved typing in editor\n\n"
    )
    assert result.exit_code == 0, result.output
    assert "Enter PASS, FAIL, or SKIP." in result.output
    summary = (output / "summary.md").read_text()
    assert "| WIN-04 | PASS | manual |" in summary
    assert "| WIN-05 | MISSING |" in summary


def test_real_git_candidate_requires_clean_worktree(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """Git metadata is read from the actual checkout and rejects uncommitted input."""
    subprocess.run(["git", "init", str(tmp_path)], check=True, capture_output=True)
    subprocess.run(
        [
            "git",
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "-c",
            f"core.hooksPath={tmp_path / 'no-hooks'}",
            "commit",
            "--allow-empty",
            "-m",
            "Test candidate",
        ],
        cwd=tmp_path,
        check=True,
        capture_output=True,
    )
    monkeypatch.chdir(tmp_path)
    candidate = runner.clean_candidate()
    assert len(candidate) == 40
    assert runner.git_output("rev-parse", "HEAD") == candidate
    runner.ensure_candidate(candidate)
    (tmp_path / "uncommitted.txt").write_text("Candidate changed")
    with pytest.raises(ValueError, match="worktree must be clean"):
        runner.clean_candidate()


def test_incomplete_template_cannot_skip_required_check(tmp_path: Path) -> None:
    """A template missing a mandatory row is rejected instead of shortening the run."""
    template = tmp_path / ".github" / "PULL_REQUEST_TEMPLATE" / "release.md"
    template.parent.mkdir(parents=True)
    template.write_text(
        "- [ ] **MAC-01** — Result: PENDING — Startup\n", encoding="utf-8"
    )
    with pytest.raises(ValueError, match="does not match the gate checklist"):
        runner.check_descriptions(tmp_path, "macos-arm64-appkit")


@pytest.mark.parametrize("colored", [False, True])
def test_module_entrypoint_displays_help_without_hardware(
    monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str], colored: bool
) -> None:
    """The executable module exposes the documented CLI without executing checks."""
    if colored:
        monkeypatch.delenv("NO_COLOR", raising=False)
        monkeypatch.setenv("FORCE_COLOR", "1")
    else:
        monkeypatch.delenv("FORCE_COLOR", raising=False)
        monkeypatch.setenv("NO_COLOR", "1")
    monkeypatch.setattr(sys, "argv", [runner.__file__, "--help"])
    with pytest.raises(SystemExit) as result:
        runpy.run_path(runner.__file__, run_name="__main__")
    assert result.value.code == 0
    output = re.sub(r"\x1b\[[0-9;]*m", "", capsys.readouterr().out)
    assert "--plan" in output
    assert "--keyboard" in output


def setup_host(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    """Provide a hardware-free Windows session for interactive CLI tests."""
    root = tmp_path / "checkout"
    root.mkdir()
    monkeypatch.setattr(runner, "git_output", lambda *args: str(root))
    monkeypatch.setattr(runner, "clean_candidate", lambda: CANDIDATE)
    monkeypatch.setattr(
        runner, "detect_platform", lambda: ("windows-x86_64-win32", "Win32")
    )
    monkeypatch.setattr(
        runner,
        "check_descriptions",
        lambda *args: {"WIN-04": "Check focus", "WIN-05": "Check labels"},
    )


def args(output: Path) -> list[str]:
    """Build arguments shared by the documented guided commands."""
    return [
        "--output",
        str(output),
        "--tester",
        "Tester",
        "--keyboard",
        "Insixty|1|abc",
    ]

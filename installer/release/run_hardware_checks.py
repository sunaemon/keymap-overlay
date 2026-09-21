# Copyright 2026 sunaemon
# SPDX-License-Identifier: MIT
import logging
import os
import platform
import subprocess
from datetime import datetime, timezone
from pathlib import Path
from typing import Annotated

import typer

from installer.release.check_hardware_gate import EXPECTED_CHECK_SECTIONS
from installer.release.collect_hardware_evidence import EvidenceRecord, collect_evidence
from installer.release.record_hardware_evidence import (
    PROFILE_RESULTS,
    EvidenceRecordError,
    build_record,
    write_record_bundle,
)
from model.src.util import initialize_logging

logger = logging.getLogger(__name__)
app = typer.Typer()

HELPERS = {
    "macos-arm64-appkit": (
        ("test-hardware-physical-reports-macos", None),
        ("test-hardware-session-macos", "macos-session"),
    ),
    "linux-x86_64-kde-wayland": (
        ("test-hardware-physical-reports-linux", None),
        ("test-hardware-session-linux", "linux-kde-session"),
    ),
    "linux-x86_64-gnome-wayland": (("test-hardware-physical-reports-linux", None),),
    "windows-x86_64-win32": (),
}


@app.command()
def main(
    output: Annotated[
        Path, typer.Option(help="New run directory outside the checkout")
    ],
    keyboards: Annotated[
        list[str],
        typer.Option(
            "--keyboard",
            help="NAME|ID|FIRMWARE_REVISION; repeat for each participating board",
        ),
    ],
    tester: Annotated[
        str, typer.Option(prompt=True, help="Person making physical observations")
    ],
    plan: Annotated[
        bool,
        typer.Option(
            help="Show the plan without running helpers or recording observations"
        ),
    ] = False,
) -> None:
    """Run existing local HIL helpers and collect remaining tester observations."""
    initialize_logging()
    try:
        run_guided_checks(output, keyboards, tester, plan=plan)
    except (OSError, ValueError, EvidenceRecordError, subprocess.SubprocessError):
        logger.exception("Guided hardware checks stopped; retain any saved transcripts")
        raise typer.Exit(code=1) from None


def run_guided_checks(
    output: Path, keyboards: list[str], tester: str, *, plan: bool
) -> None:
    """Capture one candidate's local evidence without inferring human results."""
    root = Path(git_output("rev-parse", "--show-toplevel"))
    candidate = clean_candidate()
    platform_id, session = detect_platform()
    output = output.resolve()
    validate_destination(root, output, tester)
    descriptions = check_descriptions(root, platform_id)
    helpers = HELPERS[platform_id]
    typer.echo(f"Candidate: {candidate}\nPlatform: {platform_id}\nSession: {session}")
    for target, _ in helpers:
        typer.echo(f"Helper: make {target}")
    typer.echo(
        "Remaining checks will prompt for PASS, FAIL, or SKIP and an observation."
    )
    typer.echo(
        "Firmware flashing, lifecycle operations, and login are not started automatically."
    )
    typer.echo(
        "For the typing/identity check, observe normal physical typing before helpers and again afterward."
    )
    if plan:
        return
    typer.confirm(
        "Proceed? Helpers install/restart the overlay; macOS session HIL temporarily edits "
        "Vial bindings and restores them. Close Vial, connect the listed keyboards, "
        "and complete the documented HIL prerequisites first",
        abort=True,
    )
    output.mkdir(parents=True)
    metadata = output / "run.txt"
    metadata.write_text(
        f"Candidate: {candidate}\nTester: {tester}\nPlatform: {platform_id}\n"
        f"Session: {session}\nStarted: {datetime.now(timezone.utc).isoformat()}\n"
        f"Keyboard declarations: {keyboards}\n",
        encoding="utf-8",
    )
    base = build_record(
        candidate_sha=candidate,
        platform_id=platform_id,
        os_version=platform.platform(),
        session=session,
        transcript=metadata,
        keyboards=keyboards,
        checks=[],
        lifecycle=None,
    )
    records: list[Path] = []
    completed: set[str] = set()
    finished = False
    try:
        for target, profile in helpers:
            ensure_candidate(candidate)
            transcript = output / f"{target}.txt"
            code = run_helper(target, root, transcript)
            ensure_candidate(candidate)
            record = helper_record(base, transcript, profile, code)
            save_result(record, output / f"{target}.json", records)
            if code != 0:
                raise ValueError(f"{target} exited {code}; stopped, see {transcript}")
            completed.update(check.check_id for check in record.checks)
        for check_id, description in descriptions.items():
            if check_id in completed:
                continue
            record = observe_check(base, output, metadata, check_id, description)
            if record is not None:
                save_result(record, output / f"{check_id}.json", records)
                if record.checks[0].result == "FAIL":
                    raise ValueError(f"{check_id} failed; stopped for investigation")
        ensure_candidate(candidate)
        finished = True
    finally:
        summary = collect_evidence(candidate, records)
        state = (
            "Finished prompting; review missing items"
            if finished
            else "STOPPED before completion; inspect run transcripts"
        )
        summary = f"Run status: {state}\n\n" + summary
        (output / "summary.md").write_text(summary, encoding="utf-8")
        logger.info(
            "Saved run and summary to %s; lifecycle, coverage, and skipped checks remain for review",
            output,
        )


def observe_check(
    base: EvidenceRecord, output: Path, metadata: Path, check_id: str, description: str
) -> EvidenceRecord | None:
    """Require the tester to explicitly report an observation or skip the check."""
    typer.echo(f"\n{check_id}: {description}")
    result = prompt_result()
    if result == "SKIP":
        return None
    detail = typer.prompt(
        "Describe what you performed and observed; cite supporting logs"
    )
    if not detail.strip():
        raise ValueError("An explicit observation is required")
    ensure_candidate(base.candidate_sha)
    observation = output / f"{check_id}.txt"
    observation.write_text(
        metadata.read_text(encoding="utf-8") + f"\n{check_id}: {description}\n"
        f"Result: {result}\nObservation: {detail}\n",
        encoding="utf-8",
    )
    data = base.model_dump()
    data["transcript"] = observation
    data["checks"] = [
        {
            "check_id": check_id,
            "result": result,
            "evidence_kind": "manual",
            "detail": detail,
        }
    ]
    return EvidenceRecord.model_validate(data)


def validate_destination(root: Path, output: Path, tester: str) -> None:
    """Keep evidence out of the checkout and preserve earlier run directories."""
    if output.is_relative_to(root):
        raise ValueError("Output must be outside the candidate checkout")
    if output.exists():
        raise ValueError(f"Use a new run directory: {output}")
    if not tester.strip():
        raise ValueError("Tester name must not be empty")


def prompt_result() -> str:
    """Accept only explicit outcomes, with skipping as the default."""
    while True:
        result = typer.prompt(
            "Observed result (PASS/FAIL/SKIP)", default="SKIP"
        ).upper()
        if result in {"PASS", "FAIL", "SKIP"}:
            return result
        typer.echo("Enter PASS, FAIL, or SKIP.")


def git_output(*arguments: str) -> str:
    """Read candidate repository metadata using Git."""
    return subprocess.run(
        ["git", *arguments], check=True, capture_output=True, text=True
    ).stdout.strip()


def clean_candidate() -> str:
    """Require a clean checkout before any hardware work."""
    if git_output("status", "--porcelain"):
        raise ValueError("Candidate worktree must be clean")
    return git_output("rev-parse", "HEAD")


def ensure_candidate(candidate: str) -> None:
    """Reject changes to the candidate during a test run."""
    if clean_candidate() != candidate:
        raise ValueError("Candidate changed during the run; results must be reviewed")


def detect_platform() -> tuple[str, str]:
    """Detect only the supported release architecture and desktop sessions."""
    system, machine = platform.system(), platform.machine().lower()
    if system == "Darwin" and machine == "arm64":
        return "macos-arm64-appkit", "AppKit / Aqua"
    if system == "Windows" and machine in {"amd64", "x86_64"}:
        return "windows-x86_64-win32", "Win32 / desktop"
    desktop = os.environ.get("XDG_CURRENT_DESKTOP", "").upper()
    if (
        system == "Linux"
        and machine == "x86_64"
        and os.environ.get("XDG_SESSION_TYPE") == "wayland"
    ):
        for token, name in (("KDE", "kde"), ("GNOME", "gnome")):
            if token in desktop.split(":"):
                return f"linux-x86_64-{name}-wayland", f"{desktop} / Wayland"
    raise ValueError(
        "Unsupported release architecture/session; use the manual evidence workflow"
    )


def check_descriptions(root: Path, platform_id: str) -> dict[str, str]:
    """Read current checklist wording instead of maintaining a second checklist."""
    ids = set(EXPECTED_CHECK_SECTIONS[f"{platform_id} checks"])
    if platform_id.startswith("linux-"):
        ids.update(EXPECTED_CHECK_SECTIONS["linux-x86_64 shared checks"])
    descriptions = {}
    for line in (
        (root / ".github/PULL_REQUEST_TEMPLATE/release.md")
        .read_text(encoding="utf-8")
        .splitlines()
    ):
        if line.startswith("- [ ] **"):
            check_id = line.split("**")[1]
            if check_id in ids:
                descriptions[check_id] = line.split(" — ", 2)[2]
    if descriptions.keys() != ids:
        raise ValueError("Release template does not match the gate checklist")
    return descriptions


def run_helper(target: str, root: Path, transcript: Path) -> int:
    """Stream helper output to the terminal and retain its final exit status."""
    with transcript.open("w", encoding="utf-8") as log:
        log.write(f"Command: make {target}\n")
        environment = dict(
            os.environ, KMO_HIL_LOG_DIR=str(transcript.parent / "helper-logs")
        )
        with subprocess.Popen(
            ["make", target],
            cwd=root,
            env=environment,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            text=True,
            encoding="utf-8",
            errors="replace",
        ) as process:
            assert process.stdout is not None
            for line in process.stdout:
                log.write(line)
                log.flush()
                typer.echo(line, nl=False)
            code = process.wait()
        log.write(f"Exit status: {code}\n")
    return code


def helper_record(
    base: EvidenceRecord, transcript: Path, profile: str | None, code: int
) -> EvidenceRecord:
    """Import complete results only after the helper including cleanup succeeds."""
    checks = []
    if profile is not None and code != 0:
        checks = [
            f"{check_id}|FAIL|automated|Helper exited {code}; inspect transcript"
            for check_id in PROFILE_RESULTS[profile][1]
        ]
    return build_record(
        candidate_sha=base.candidate_sha,
        platform_id=base.platform_id,
        os_version=base.os_version,
        session=base.session,
        transcript=transcript,
        keyboards=[
            f"{board.name}|{board.keyboard_id}|{board.firmware_revision}"
            for board in base.keyboards
        ],
        checks=checks,
        lifecycle=None,
        profile=profile if code == 0 else None,
    )


def save_result(record: EvidenceRecord, output: Path, records: list[Path]) -> None:
    """Save each completed result immediately so later interruptions retain it."""
    write_record_bundle(record, output)
    records.append(output)


if __name__ == "__main__":
    app()

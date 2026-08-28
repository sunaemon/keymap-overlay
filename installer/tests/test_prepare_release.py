# Copyright 2026 sunaemon
# SPDX-License-Identifier: MIT
import json
import subprocess
from pathlib import Path

import pytest

from installer.release.check_hardware_gate import (
    EXPECTED_CHECK_SECTIONS,
    changed_files_command,
)
from installer.release.prepare_release import (
    ReleasePlan,
    ReleasePreparationError,
    cargo_metadata_command,
    commit_tree_command,
    prepare_release,
    pull_requests_command,
    read_project_version,
    release_command,
    tag_command,
    write_github_output,
)

REPOSITORY = "sunaemon/keymap-overlay"
TESTED_SHA = "tested"
BASE_SHA = "base"
HEAD_SHA = "a" * 40
TREE_SHA = "c" * 40


class FakeRunner:
    """Return canned subprocess results for release preparation commands."""

    def __init__(
        self,
        *,
        current: str = "0.0.5",
        previous: str = "0.0.4",
        evidence_tree: str = TREE_SHA,
        published_tree: str = TREE_SHA,
    ) -> None:
        packages = [{"name": "keymap-overlay", "version": current}]
        pull_requests = [
            {
                "number": 42,
                "base": {"ref": "main", "sha": BASE_SHA},
                "head": {"ref": "release", "sha": HEAD_SHA},
                "body": complete_gate(),
                "merged_at": "2026-08-16T00:00:00Z",
            }
        ]
        self.results = {
            tuple(cargo_metadata_command()): completed(
                stdout=json.dumps({"packages": packages})
            ),
            tuple(pull_requests_command(REPOSITORY, TESTED_SHA)): completed(
                stdout=json.dumps(pull_requests)
            ),
            ("git", "show", f"{BASE_SHA}:pyproject.toml"): completed(
                stdout=project(previous)
            ),
            tuple(changed_files_command(BASE_SHA, TESTED_SHA)): completed(),
            tuple(commit_tree_command(REPOSITORY, HEAD_SHA)): completed(
                stdout=evidence_tree
            ),
            tuple(commit_tree_command(REPOSITORY, TESTED_SHA)): completed(
                stdout=published_tree
            ),
            tuple(release_command(REPOSITORY, f"v{current}")): not_found(),
            tuple(tag_command(REPOSITORY, f"v{current}")): not_found(),
        }

    def __call__(
        self, command: list[str], check: bool
    ) -> subprocess.CompletedProcess[str]:
        result = self.results[tuple(command)]
        if check and result.returncode != 0:
            raise subprocess.CalledProcessError(result.returncode, command)
        return result


def test_a_version_bump_from_a_merged_pr_prepares_a_release(tmp_path: Path) -> None:
    plan = prepare_release(
        TESTED_SHA,
        REPOSITORY,
        project_file=write_project(tmp_path, "0.0.5"),
        runner=FakeRunner(),
    )

    assert plan == ReleasePlan(should_release=True, tag="v0.0.5")


def test_a_merge_without_a_version_bump_is_skipped(tmp_path: Path) -> None:
    plan = prepare_release(
        TESTED_SHA,
        REPOSITORY,
        project_file=write_project(tmp_path, "0.0.4"),
        runner=FakeRunner(current="0.0.4", previous="0.0.4"),
    )

    assert plan == ReleasePlan(should_release=False)


def test_a_merge_with_content_different_from_the_tested_head_is_rejected(
    tmp_path: Path,
) -> None:
    """A merge commit cannot publish a tree different from the tested PR head."""
    with pytest.raises(ReleasePreparationError, match="differs from tested pull"):
        prepare_release(
            TESTED_SHA,
            REPOSITORY,
            project_file=write_project(tmp_path, "0.0.5"),
            runner=FakeRunner(published_tree="d" * 40),
        )


def test_a_version_bump_without_hardware_evidence_is_rejected(tmp_path: Path) -> None:
    """A merged version bump without hardware evidence is rejected."""
    runner = FakeRunner()
    pull_requests = json.loads(
        runner.results[tuple(pull_requests_command(REPOSITORY, TESTED_SHA))].stdout
    )
    pull_requests[0]["body"] = None
    runner.results[tuple(pull_requests_command(REPOSITORY, TESTED_SHA))] = completed(
        stdout=json.dumps(pull_requests)
    )

    with pytest.raises(ReleasePreparationError, match="hardware release gate failed"):
        prepare_release(
            TESTED_SHA,
            REPOSITORY,
            project_file=write_project(tmp_path, "0.0.5"),
            runner=runner,
        )


def test_firmware_change_cannot_use_global_na_evidence(tmp_path: Path) -> None:
    """Release preparation derives required firmware evidence from the merge diff."""
    runner = FakeRunner()
    pull_requests = json.loads(
        runner.results[tuple(pull_requests_command(REPOSITORY, TESTED_SHA))].stdout
    )
    pull_requests[0]["body"] = (
        complete_gate()
        .replace(
            "GLOBAL-01** — Result: PASS",
            "GLOBAL-01** — Result: N/A: no flash run",
        )
        .replace(
            "GLOBAL-02** — Result: PASS",
            "GLOBAL-02** — Result: N/A: no EEPROM run",
        )
    )
    runner.results[tuple(pull_requests_command(REPOSITORY, TESTED_SHA))] = completed(
        stdout=json.dumps(pull_requests)
    )
    runner.results[tuple(changed_files_command(BASE_SHA, TESTED_SHA))] = completed(
        stdout="firmware/layer_notify.h\0"
    )

    with pytest.raises(ReleasePreparationError, match="GLOBAL-01, GLOBAL-02"):
        prepare_release(
            TESTED_SHA,
            REPOSITORY,
            project_file=write_project(tmp_path, "0.0.5"),
            runner=runner,
        )


def test_an_existing_release_is_skipped(tmp_path: Path) -> None:
    """An existing release bypasses duplicate gate and publication work."""
    runner = FakeRunner()
    runner.results[tuple(release_command(REPOSITORY, "v0.0.5"))] = completed()
    pull_requests = json.loads(
        runner.results[tuple(pull_requests_command(REPOSITORY, TESTED_SHA))].stdout
    )
    pull_requests[0]["body"] = None
    runner.results[tuple(pull_requests_command(REPOSITORY, TESTED_SHA))] = completed(
        stdout=json.dumps(pull_requests)
    )

    plan = prepare_release(
        TESTED_SHA,
        REPOSITORY,
        project_file=write_project(tmp_path, "0.0.5"),
        runner=runner,
    )

    assert plan == ReleasePlan(should_release=False)


def test_a_manually_created_tag_is_rejected(tmp_path: Path) -> None:
    runner = FakeRunner()
    runner.results[tuple(tag_command(REPOSITORY, "v0.0.5"))] = completed()

    with pytest.raises(ReleasePreparationError, match="tags must be created"):
        prepare_release(
            TESTED_SHA,
            REPOSITORY,
            project_file=write_project(tmp_path, "0.0.5"),
            runner=runner,
        )


def test_a_commit_without_one_merged_main_pr_is_rejected(tmp_path: Path) -> None:
    runner = FakeRunner()
    runner.results[tuple(pull_requests_command(REPOSITORY, TESTED_SHA))] = completed(
        stdout="[]"
    )
    runner.results[("git", "show", f"{TESTED_SHA}^:pyproject.toml")] = completed(
        stdout=project("0.0.4")
    )

    with pytest.raises(
        ReleasePreparationError, match="not the merge of a pull request"
    ):
        prepare_release(
            TESTED_SHA,
            REPOSITORY,
            project_file=write_project(tmp_path, "0.0.5"),
            runner=runner,
        )


def test_a_non_release_direct_push_is_skipped(tmp_path: Path) -> None:
    runner = FakeRunner(current="0.0.4")
    runner.results[tuple(pull_requests_command(REPOSITORY, TESTED_SHA))] = completed(
        stdout="[]"
    )
    runner.results[("git", "show", f"{TESTED_SHA}^:pyproject.toml")] = completed(
        stdout=project("0.0.4")
    )

    plan = prepare_release(
        TESTED_SHA,
        REPOSITORY,
        project_file=write_project(tmp_path, "0.0.4"),
        runner=runner,
    )

    assert plan == ReleasePlan(should_release=False)


def test_mismatched_cargo_versions_are_rejected(tmp_path: Path) -> None:
    runner = FakeRunner()
    runner.results[tuple(cargo_metadata_command())] = completed(
        stdout=json.dumps({"packages": [{"name": "keymap-core", "version": "0.0.4"}]})
    )

    with pytest.raises(ReleasePreparationError, match="keymap-core=0.0.4"):
        prepare_release(
            TESTED_SHA,
            REPOSITORY,
            project_file=write_project(tmp_path, "0.0.5"),
            runner=runner,
        )


def test_non_404_github_errors_are_not_treated_as_missing(tmp_path: Path) -> None:
    runner = FakeRunner()
    runner.results[tuple(release_command(REPOSITORY, "v0.0.5"))] = completed(
        returncode=1, stderr="network unavailable"
    )

    with pytest.raises(ReleasePreparationError, match="network unavailable"):
        prepare_release(
            TESTED_SHA,
            REPOSITORY,
            project_file=write_project(tmp_path, "0.0.5"),
            runner=runner,
        )


def test_github_outputs_are_appended(tmp_path: Path) -> None:
    output = tmp_path / "github-output"

    write_github_output(output, ReleasePlan(should_release=True, tag="v0.0.5"))

    assert output.read_text() == "should_release=true\ntag=v0.0.5\n"


def test_invalid_project_metadata_is_rejected() -> None:
    with pytest.raises(ReleasePreparationError, match="project version"):
        read_project_version(b"[project]\nname = 'keymap-overlay'\n")


def write_project(tmp_path: Path, version: str) -> Path:
    """Write minimal project metadata for a release preparation test."""
    path = tmp_path / "pyproject.toml"
    path.write_text(project(version))
    return path


def project(version: str) -> str:
    """Return minimal project metadata using a version."""
    return f'[project]\nname = "keymap-overlay"\nversion = "{version}"\n'


def complete_gate() -> str:
    """Return complete hardware evidence for the merged pull request."""
    check_sections = "\n".join(
        "### "
        + heading
        + "\n\n"
        + "".join(
            f"- [x] **{check_id}** — Result: PASS — passed\n" for check_id in check_ids
        )
        for heading, check_ids in EXPECTED_CHECK_SECTIONS.items()
    )
    return f"""Candidate commit: `{HEAD_SHA}`

### Platform test matrix

| Platform ID | Architecture | OS version | Desktop / session | Keyboard(s) | `KEYBOARD_ID(s)` | Firmware revision(s) |
| ----------- | ------------ | ---------- | ----------------- | ----------- | ---------------- | -------------------- |
| macos-arm64-appkit | arm64 | macOS 15.6 | AppKit / Aqua | Insixty | 1 | stable firmware |
| linux-x86_64-kde-wayland | x86_64 | Fedora 42 | KDE Plasma 6 / Wayland | DOIO KB16 | 2 | stable firmware |
| linux-x86_64-gnome-wayland | x86_64 | Ubuntu 26.04 | GNOME 49 / Wayland | Insixty | 1 | stable firmware |
| linux-arm64-kde-wayland | arm64 | Fedora 42 | KDE Plasma 6 / Wayland | Insixty | 1 | stable firmware |
| windows-x86_64-wpf | x86_64 | Windows 11 24H2 | WPF / desktop | Insixty, DOIO KB16 | 1, 2 | stable firmware |
| windows-arm64-wpf | arm64 | Windows 11 24H2 | WPF / desktop | Insixty | 1 | stable firmware |

### Keyboard coverage

| Coverage ID | Keyboard(s) | `KEYBOARD_ID(s)` | Platform ID(s) | Result |
| ----------- | ----------- | ---------------- | -------------- | ------ |
| bundled-keyboards | Insixty, DOIO KB16 | 1, 2 | macos-arm64-appkit, linux-x86_64-kde-wayland | PASS |
| encoder-keyboard | DOIO KB16 | 2 | linux-x86_64-kde-wayland | PASS |
| simultaneous-keyboards | Insixty, DOIO KB16 | 1, 2 | windows-x86_64-wpf | PASS |

{check_sections}
### Lifecycle results

| Platform ID | Upgrade | Rollback | Uninstall | Evidence |
| ----------- | ------- | -------- | --------- | -------- |
| macos-arm64-appkit | PASS | PASS | PASS | local acceptance log |
| linux-x86_64-kde-wayland | PASS | PASS | PASS | local acceptance log |
| linux-x86_64-gnome-wayland | PASS | PASS | PASS | local acceptance log |
| linux-arm64-kde-wayland | PASS | PASS | PASS | local acceptance log |
| windows-x86_64-wpf | PASS | PASS | PASS | local acceptance log |
| windows-arm64-wpf | PASS | PASS | PASS | local acceptance log |
"""


def completed(
    *, stdout: str = "", stderr: str = "", returncode: int = 0
) -> subprocess.CompletedProcess[str]:
    """Return a canned subprocess result."""
    return subprocess.CompletedProcess([], returncode, stdout, stderr)


def not_found() -> subprocess.CompletedProcess[str]:
    """Return a canned GitHub API not-found response."""
    return completed(returncode=1, stderr="gh: Not Found (HTTP 404)")

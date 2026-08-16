# Copyright 2026 sunaemon
# SPDX-License-Identifier: MIT
import json
import subprocess
from pathlib import Path

import pytest

from installer.release.prepare_release import (
    ReleasePlan,
    ReleasePreparationError,
    cargo_metadata_command,
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


class FakeRunner:
    """Return canned subprocess results for release preparation commands."""

    def __init__(self, *, current: str = "0.0.5", previous: str = "0.0.4") -> None:
        packages = [{"name": "keymap-overlay", "version": current}]
        pull_requests = [
            {
                "number": 42,
                "base": {"ref": "main", "sha": BASE_SHA},
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


def test_an_existing_release_is_skipped(tmp_path: Path) -> None:
    runner = FakeRunner()
    runner.results[tuple(release_command(REPOSITORY, "v0.0.5"))] = completed()

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


def completed(
    *, stdout: str = "", stderr: str = "", returncode: int = 0
) -> subprocess.CompletedProcess[str]:
    """Return a canned subprocess result."""
    return subprocess.CompletedProcess([], returncode, stdout, stderr)


def not_found() -> subprocess.CompletedProcess[str]:
    """Return a canned GitHub API not-found response."""
    return completed(returncode=1, stderr="gh: Not Found (HTTP 404)")

# Copyright 2026 sunaemon
# SPDX-License-Identifier: MIT
import json
import logging
import subprocess
import tomllib
from collections.abc import Callable
from dataclasses import dataclass
from pathlib import Path
from typing import Annotated, TypeVar

import typer
from pydantic import BaseModel, TypeAdapter, ValidationError

from src.util import initialize_logging

logger = logging.getLogger(__name__)

app = typer.Typer()

CommandRunner = Callable[[list[str], bool], subprocess.CompletedProcess[str]]
ValidatedJson = TypeVar("ValidatedJson")


class ReleasePreparationError(Exception):
    """Raised when a version bump cannot be published safely."""


@dataclass(frozen=True)
class PullRequest:
    """A merged pull request associated with the tested commit."""

    number: int
    base_sha: str


@dataclass(frozen=True)
class ReleasePlan:
    """The release decision written to the GitHub Actions job outputs."""

    should_release: bool
    tag: str | None = None


class CargoPackage(BaseModel):
    """A package entry returned by cargo metadata."""

    name: str
    version: str


class CargoMetadata(BaseModel):
    """The cargo metadata fields used to validate release versions."""

    packages: list[CargoPackage]


class PullRequestBase(BaseModel):
    """The base branch metadata returned for a GitHub pull request."""

    ref: str
    sha: str


class GitHubPullRequest(BaseModel):
    """The GitHub pull request fields used to identify a release merge."""

    number: int
    base: PullRequestBase
    merged_at: str | None


CARGO_METADATA_ADAPTER = TypeAdapter(CargoMetadata)
PULL_REQUESTS_ADAPTER = TypeAdapter(list[GitHubPullRequest])


@app.command()
def main(
    tested_sha: Annotated[str, typer.Option(help="Commit whose CI run passed")],
    repository: Annotated[
        str, typer.Option(help="GitHub repository in owner/name form")
    ],
    github_output: Annotated[
        Path, typer.Option(help="GitHub Actions output file to append to")
    ],
) -> None:
    """Prepare a release only for a tested version-bump PR merge."""
    initialize_logging()
    try:
        plan = prepare_release(tested_sha, repository)
        write_github_output(github_output, plan)
    except (OSError, ReleasePreparationError, subprocess.SubprocessError):
        logger.exception("Failed to prepare the release for %s", tested_sha)
        raise typer.Exit(code=1) from None


def prepare_release(
    tested_sha: str,
    repository: str,
    *,
    project_file: Path = Path("pyproject.toml"),
    runner: CommandRunner | None = None,
) -> ReleasePlan:
    """Return the release plan for a successful main-branch CI commit."""
    run = runner or run_command
    current_version = read_project_version(project_file.read_bytes())
    validate_cargo_versions(
        current_version,
        _run_json(run, cargo_metadata_command(), CARGO_METADATA_ADAPTER),
    )

    pull_request = find_merged_pull_request(
        _run_json(
            run,
            pull_requests_command(repository, tested_sha),
            PULL_REQUESTS_ADAPTER,
        )
    )
    comparison_sha = (
        pull_request.base_sha if pull_request is not None else f"{tested_sha}^"
    )
    previous_project = run(
        ["git", "show", f"{comparison_sha}:pyproject.toml"], True
    ).stdout.encode()
    previous_version = read_project_version(previous_project)
    if current_version == previous_version:
        logger.info(
            "The tested merge does not change the project version; skipping release"
        )
        return ReleasePlan(should_release=False)
    if pull_request is None:
        raise ReleasePreparationError(
            "The version-changing commit is not the merge of a pull request into main"
        )

    tag = f"v{current_version}"
    if github_resource_exists(run, release_command(repository, tag)):
        logger.info("Release %s already exists; skipping duplicate publication", tag)
        return ReleasePlan(should_release=False)
    if github_resource_exists(run, tag_command(repository, tag)):
        raise ReleasePreparationError(
            f"Tag {tag} already exists without a release; "
            "tags must be created by the release workflow"
        )

    logger.info(
        "Pull request #%d bumps %s to %s; preparing %s",
        pull_request.number,
        previous_version,
        current_version,
        tag,
    )
    return ReleasePlan(should_release=True, tag=tag)


def write_github_output(path: Path, plan: ReleasePlan) -> None:
    """Append a release plan to a GitHub Actions output file."""
    with path.open("a", encoding="utf-8") as output:
        output.write(f"should_release={str(plan.should_release).lower()}\n")
        if plan.tag is not None:
            output.write(f"tag={plan.tag}\n")


def read_project_version(project: bytes) -> str:
    """Read the package version from pyproject.toml content."""
    try:
        version = tomllib.loads(project.decode())["project"]["version"]
    except (KeyError, TypeError, UnicodeDecodeError, tomllib.TOMLDecodeError) as error:
        raise ReleasePreparationError(
            "pyproject.toml has no valid project version"
        ) from error
    if not isinstance(version, str) or not version:
        raise ReleasePreparationError("pyproject.toml has no valid project version")
    return version


def validate_cargo_versions(version: str, metadata: CargoMetadata) -> None:
    """Require every Cargo workspace package to use the Python version."""
    mismatches = [
        f"{package.name}={package.version}"
        for package in metadata.packages
        if package.version != version
    ]
    if mismatches:
        raise ReleasePreparationError(
            "Cargo package versions do not match Python "
            f"{version}: {', '.join(mismatches)}"
        )


def find_merged_pull_request(
    response: list[GitHubPullRequest],
) -> PullRequest | None:
    """Return the merged main pull request associated with a commit, if any."""
    matches = [
        pull_request
        for pull_request in response
        if pull_request.base.ref == "main" and pull_request.merged_at is not None
    ]
    if len(matches) > 1:
        raise ReleasePreparationError(
            "The tested commit is associated with more than one merged "
            "pull request into main"
        )
    if not matches:
        return None
    return PullRequest(number=matches[0].number, base_sha=matches[0].base.sha)


def github_resource_exists(run: CommandRunner, command: list[str]) -> bool:
    """Return whether a GitHub API resource exists, failing on non-404 errors."""
    result = run(command, False)
    if result.returncode == 0:
        return True
    if "HTTP 404" in result.stderr:
        return False
    raise ReleasePreparationError(result.stderr.strip() or "GitHub API request failed")


def run_command(command: list[str], check: bool) -> subprocess.CompletedProcess[str]:
    """Run a release preparation command and capture its text output."""
    return subprocess.run(command, check=check, capture_output=True, text=True)


def cargo_metadata_command() -> list[str]:
    """Return the command that reads all Cargo workspace package versions."""
    return ["cargo", "metadata", "--no-deps", "--format-version", "1"]


def pull_requests_command(repository: str, tested_sha: str) -> list[str]:
    """Return the command that finds pull requests associated with a commit."""
    return ["gh", "api", f"repos/{repository}/commits/{tested_sha}/pulls"]


def release_command(repository: str, tag: str) -> list[str]:
    """Return the command that looks up a GitHub release by tag."""
    return ["gh", "api", f"repos/{repository}/releases/tags/{tag}"]


def tag_command(repository: str, tag: str) -> list[str]:
    """Return the command that looks up a Git tag reference."""
    return ["gh", "api", f"repos/{repository}/git/ref/tags/{tag}"]


def _run_json(
    run: CommandRunner,
    command: list[str],
    adapter: TypeAdapter[ValidatedJson],
) -> ValidatedJson:
    """Run a command and decode its JSON output."""
    try:
        return adapter.validate_python(json.loads(run(command, True).stdout))
    except (json.JSONDecodeError, ValidationError) as error:
        raise ReleasePreparationError(
            f"Command returned invalid JSON: {' '.join(command)}"
        ) from error


if __name__ == "__main__":
    app()

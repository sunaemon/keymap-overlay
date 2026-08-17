# Copyright 2026 sunaemon
# SPDX-License-Identifier: MIT
import pytest

from tools.check_commit_message import SUBJECT_MAX_LENGTH, check_commit_message


def test_a_plain_subject_is_accepted() -> None:
    assert check_commit_message("Fix layer transparency resolution\n") == []


def test_a_subject_and_body_are_accepted() -> None:
    message = "Introduce rust\n\nThe overlay replaces the Hammerspoon integration.\n"

    assert check_commit_message(message) == []


@pytest.mark.parametrize(
    "subject",
    [
        "Refactor",
        "Fix README",
        "feat: install overlay as login service",
        "Fix layer transparency resolution, and clean up after the Rust migration",
    ],
)
def test_existing_history_still_passes(subject: str) -> None:
    """The rules describe the style already in use, not a new one."""
    assert check_commit_message(f"{subject}\n") == []


def test_a_subject_at_the_limit_is_accepted() -> None:
    assert check_commit_message("A" * SUBJECT_MAX_LENGTH) == []


def test_a_subject_over_the_limit_is_rejected() -> None:
    problems = check_commit_message("A" * (SUBJECT_MAX_LENGTH + 1))

    assert problems == [
        f"The subject is {SUBJECT_MAX_LENGTH + 1} characters long; "
        f"keep it within {SUBJECT_MAX_LENGTH}."
    ]


def test_a_trailing_period_is_rejected() -> None:
    assert check_commit_message("Fix README.\n") == [
        "The subject ends with a period; drop it."
    ]


def test_a_body_without_a_blank_line_is_rejected() -> None:
    """Without the blank line git treats the whole run of text as the subject."""
    message = "Introduce rust\nThe overlay replaces Hammerspoon.\n"

    assert check_commit_message(message) == [
        "Leave a blank line between the subject and the body."
    ]


def test_every_problem_is_reported_at_once() -> None:
    message = f"{'A' * (SUBJECT_MAX_LENGTH + 1)}.\nBody\n"

    assert len(check_commit_message(message)) == 3


@pytest.mark.parametrize(
    "message",
    ["", "\n\n", "# Please enter the commit message for your changes.\n"],
)
def test_an_empty_message_is_rejected(message: str) -> None:
    assert check_commit_message(message) == ["The commit message is empty."]


def test_git_comments_are_ignored() -> None:
    """Git appends the status block to the message file before the hook runs."""
    message = (
        "Fix README\n"
        "\n"
        "# Please enter the commit message for your changes.\n"
        "# On branch main\n"
        "# Changes to be committed:\n"
        "#\tmodified:   README.md\n"
    )

    assert check_commit_message(message) == []


def test_the_verbose_diff_is_not_treated_as_the_body() -> None:
    """`git commit --verbose` puts a real diff below the scissors line."""
    message = (
        "Fix README\n"
        "\n"
        "# ------------------------ >8 ------------------------\n"
        "# Do not modify or remove the line above.\n"
        "diff --git a/README.md b/README.md\n"
        "+A line that is far too long to pass as a commit subject, and ends in a period.\n"
    )

    assert check_commit_message(message) == []


def test_leading_blank_lines_do_not_hide_the_subject() -> None:
    """Git strips them, so the second line is the real subject."""
    assert check_commit_message("\n\nFix README.\n") == [
        "The subject ends with a period; drop it."
    ]


@pytest.mark.parametrize(
    "subject",
    [
        "Merge pull request #1 from sunaemon/refactor",
        "Merge branch 'main' into rust.",
        'Revert "Introduce rust, which turned out to need more work than expected."',
        # What git writes when you revert a revert.
        'Reapply "Fix layer transparency resolution, and clean up after Rust"',
        "fixup! Fix layer transparency resolution",
        "squash! Fix layer transparency resolution",
    ],
)
def test_git_generated_subjects_are_left_alone(subject: str) -> None:
    """Git writes these itself; rejecting them would block merges and rebases."""
    assert check_commit_message(f"{subject}\n") == []

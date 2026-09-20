# Copyright 2026 sunaemon
# SPDX-License-Identifier: MIT
from pathlib import Path

from tools.run_dev_task import build_command


def test_windows_hooks_use_the_native_powershell_workflow() -> None:
    """Keep Windows hooks on the native PowerShell workflow."""
    root = Path("C:/src/keymap-overlay")

    command = build_command("test-rust", [], root=root, platform="win32")

    assert command[:2] == ["powershell.exe", "-NoProfile"]
    assert str(root / "tools" / "windows.ps1") in command
    assert command[-2:] == ["-Task", "test-rust"]
    assert "make" not in command


def test_posix_hooks_keep_the_make_workflow() -> None:
    """Keep the established Make entry point on macOS and Linux."""
    command = build_command(
        "check-licenses", [], root=Path("/src/keymap-overlay"), platform="linux"
    )

    assert command == ["make", "check-licenses"]


def test_commit_message_path_is_forwarded_in_each_shell_dialect() -> None:
    """Pass Git's commit message path without shell interpolation."""
    root = Path("C:/src/keymap-overlay")
    message = "C:/src/keymap-overlay/.git/COMMIT_EDITMSG"

    command = build_command(
        "check-commit-message", [message], root=root, platform="win32"
    )

    assert command[-2:] == ["-CommitMessageFile", message]

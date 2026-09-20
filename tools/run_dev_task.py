# Copyright 2026 sunaemon
# SPDX-License-Identifier: MIT
"""Runs one repository hook task through the host-native workflow."""

import subprocess
import sys
from pathlib import Path


def main() -> None:
    """Runs the requested task through PowerShell on Windows or Make elsewhere."""
    if len(sys.argv) < 2:
        raise SystemExit("usage: run_dev_task.py TASK [ARGUMENTS...]")
    task, *arguments = sys.argv[1:]
    root = Path(__file__).parents[1]
    command = build_command(task, arguments, root=root, platform=sys.platform)
    raise SystemExit(subprocess.run(command, cwd=root, check=False).returncode)


def build_command(
    task: str, arguments: list[str], *, root: Path, platform: str
) -> list[str]:
    """Builds the host-native command for one hook task."""
    if platform == "win32":
        command = [
            "powershell.exe",
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
            str(root / "tools" / "windows.ps1"),
            "-Task",
            task,
        ]
        if task == "check-commit-message":
            command.extend(["-CommitMessageFile", *arguments])
        elif arguments:
            raise SystemExit(f"{task} does not accept arguments")
    else:
        command = ["make", task]
        if task == "check-commit-message":
            command.append(f"COMMIT_MSG_FILE={arguments[0]}")
        elif arguments:
            raise SystemExit(f"{task} does not accept arguments")
    return command


if __name__ == "__main__":
    main()

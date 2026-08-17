# Copyright 2026 sunaemon
# SPDX-License-Identifier: MIT
import logging
import os
import subprocess
import time
from pathlib import Path
from typing import Annotated

import typer

from model.src.util import initialize_logging

logger = logging.getLogger(__name__)

app = typer.Typer()

# firmware/vendor/vial-qmk/util/uf2conv.py only deploys to volumes mounted under
# /media, /media/$USER or /run/media/$USER, and only when the volume holds
# this file. A volume mounted anywhere else is invisible to it.
UF2_INFO_FILE = "INFO_UF2.TXT"
UF2_MOUNT_ROOT = Path("/run/media")
DEVICE_BY_LABEL_DIR = Path("/dev/disk/by-label")
POLL_INTERVAL_SECONDS = 0.2


class Uf2VolumeError(RuntimeError):
    """Raised when the UF2 bootloader volume cannot be made deployable."""


@app.command()
def main(
    label: Annotated[
        str, typer.Option(help="Filesystem label of the bootloader volume")
    ] = "RPI-RP2",
    timeout: Annotated[
        float, typer.Option(help="Seconds to wait for the volume to appear")
    ] = 120.0,
    sudo: Annotated[
        str, typer.Option(help="Command used to elevate, empty to never elevate")
    ] = "sudo",
    unmount_after: Annotated[
        bool,
        typer.Option("--unmount", help="Unmount the volume instead of mounting it"),
    ] = False,
) -> None:
    """Mount or unmount a UF2 bootloader volume where qmk looks for it."""
    initialize_logging()
    operation = "unmount" if unmount_after else "mount"
    try:
        if unmount_after:
            unmount_uf2_volume(label=label, sudo=sudo)
        else:
            print(mount_uf2_volume(label=label, timeout=timeout, sudo=sudo))
    except Exception:
        logger.exception("Failed to %s the %s volume", operation, label)
        raise typer.Exit(code=1) from None


def mount_uf2_volume(label: str, timeout: float, sudo: str) -> Path:
    """Return a mount point for the labelled volume that qmk can deploy to."""
    device = wait_for_device(label, timeout)
    logger.info("Bootloader volume %s appeared at %s", label, device)

    mount_point = mount_point_of(device)
    if mount_point is not None:
        if is_deployable(mount_point):
            logger.info("Already mounted at %s", mount_point)
            return mount_point
        # udisks mounts under the *authenticating* user, so driving the flash
        # over SSH lands it in /run/media/root, which qmk cannot read.
        logger.info("Remounting: %s is not writable by this user", mount_point)
        unmount(mount_point, sudo)

    target = target_mount_point(label)
    release_stale_mount(target, sudo)
    mount(device, target, sudo)

    if not is_deployable(target):
        raise Uf2VolumeError(
            f"Mounted {device} at {target} but it holds no writable {UF2_INFO_FILE}"
        )

    logger.info("Mounted %s at %s", device, target)
    return target


def unmount_uf2_volume(label: str, sudo: str) -> None:
    """Unmount the labelled UF2 volume when it remains mounted after flashing."""
    target = target_mount_point(label)
    if source_of_mount_point(target) is None:
        logger.info("Bootloader volume %s is already unmounted", label)
        return
    unmount(target, sudo)
    logger.info("Unmounted bootloader volume %s from %s", label, target)


def wait_for_device(label: str, timeout: float) -> Path:
    """Return the device with the given filesystem label, waiting for it."""
    deadline = time.monotonic() + timeout
    while True:
        device = device_for_label(label)
        if device is not None:
            return device
        if time.monotonic() >= deadline:
            raise Uf2VolumeError(
                f"No volume labelled {label} appeared within {timeout:g}s; "
                "put the board into its bootloader while this waits"
            )
        time.sleep(POLL_INTERVAL_SECONDS)


def device_for_label(label: str) -> Path | None:
    """Return the block device carrying a filesystem label, if it is present."""
    by_label = DEVICE_BY_LABEL_DIR / label
    if not by_label.exists():
        return None
    return by_label.resolve()


def mount_point_of(device: Path) -> Path | None:
    """Return where a device is mounted, or None when it is not mounted."""
    output = run(
        ["findmnt", "--noheadings", "--raw", "--output", "TARGET", str(device)]
    )
    if output is None:
        return None
    first_line = output.strip().splitlines()
    return Path(first_line[0]) if first_line else None


def source_of_mount_point(mount_point: Path) -> Path | None:
    """Return the device mounted at a path, or None when nothing is."""
    output = run(
        ["findmnt", "--noheadings", "--raw", "--output", "SOURCE", str(mount_point)]
    )
    if output is None:
        return None
    first_line = output.strip().splitlines()
    return Path(first_line[0]) if first_line else None


def is_deployable(mount_point: Path) -> bool:
    """Report whether qmk could write its UF2 into this mount point."""
    try:
        # A volume mounted by root is 0750, so even the stat raises here; that
        # is an unusable mount point rather than an error.
        return (mount_point / UF2_INFO_FILE).is_file() and os.access(
            mount_point, os.W_OK
        )
    except OSError:
        return False


def target_mount_point(label: str) -> Path:
    """Return the mount point qmk searches for this user."""
    return UF2_MOUNT_ROOT / current_user() / label


def current_user() -> str:
    """Return the user name uf2conv.py builds its search paths from."""
    user = os.environ.get("USER")
    if not user:
        raise Uf2VolumeError("USER is unset, so qmk cannot find the mounted volume")
    return user


def release_stale_mount(mount_point: Path, sudo: str) -> None:
    """Unmount whatever is left at the target from a previous flash."""
    if source_of_mount_point(mount_point) is not None:
        logger.info("Unmounting a leftover volume at %s", mount_point)
        unmount(mount_point, sudo)


def mount(device: Path, target: Path, sudo: str) -> None:
    """Mount a device so that this user, and so qmk, may write to it."""
    # vfat has no ownership of its own: without uid/gid every file belongs to
    # whoever ran mount, which is root here.
    options = f"uid={os.getuid()},gid={os.getgid()}"
    check_call(elevate(["mkdir", "-p", str(target)], sudo))
    check_call(elevate(["mount", "-o", options, str(device), str(target)], sudo))


def unmount(mount_point: Path, sudo: str) -> None:
    """Unmount a mount point, tolerating a board that already disconnected."""
    if run(elevate(["umount", str(mount_point)], sudo)) is not None:
        return
    if source_of_mount_point(mount_point) is None:
        logger.info("Bootloader volume at %s disconnected before unmount", mount_point)
        return
    raise Uf2VolumeError(f"Could not unmount {mount_point}")


def elevate(command: list[str], sudo: str) -> list[str]:
    """Prefix a command with the sudo command unless it is already root."""
    if not sudo or os.geteuid() == 0:
        return command
    return [*sudo.split(), *command]


def check_call(command: list[str]) -> None:
    """Run a command, raising with its output when it fails."""
    result = subprocess.run(command, capture_output=True, text=True, check=False)
    if result.returncode != 0:
        raise Uf2VolumeError(
            f"{' '.join(command)} failed with status {result.returncode}: "
            f"{result.stderr.strip()}"
        )


def run(command: list[str]) -> str | None:
    """Run a command, returning its stdout or None when it fails."""
    result = subprocess.run(command, capture_output=True, text=True, check=False)
    if result.returncode != 0:
        return None
    return result.stdout


if __name__ == "__main__":
    app()

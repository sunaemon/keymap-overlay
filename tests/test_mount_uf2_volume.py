# Copyright 2026 sunaemon
# SPDX-License-Identifier: MIT
import sys
from pathlib import Path

import pytest

from scripts import mount_uf2_volume
from scripts.mount_uf2_volume import (
    Uf2VolumeError,
    elevate,
    is_deployable,
    target_mount_point,
    wait_for_device,
)
from scripts.mount_uf2_volume import (
    mount_uf2_volume as mount_volume,
)

# The module under test mounts the UF2 bootloader volume on Linux, and reaches
# for os.getuid, os.geteuid, chmod permissions and findmnt to do it. None of
# those behave on Windows — patching os.geteuid there raises AttributeError
# before a test can run — and there is nothing to cover either way, because
# Windows does not flash firmware at all.
pytestmark = pytest.mark.skipif(
    sys.platform == "win32",
    reason="mount_uf2_volume is Linux-only; Windows does not flash firmware",
)


def deployable(path: Path) -> Path:
    """Make a directory look like a mounted UF2 bootloader volume."""
    path.mkdir(parents=True, exist_ok=True)
    (path / "INFO_UF2.TXT").write_text("Model: Raspberry Pi RP2\n")
    return path


def test_a_volume_without_the_info_file_is_not_deployable(tmp_path: Path) -> None:
    """qmk filters on INFO_UF2.TXT, so a bare directory would be skipped."""
    assert not is_deployable(tmp_path)
    assert is_deployable(deployable(tmp_path / "RPI-RP2"))


def test_a_volume_this_user_cannot_read_is_not_deployable(tmp_path: Path) -> None:
    """A root-owned mount is 0750, so even stat-ing through it raises."""
    unreadable = deployable(tmp_path / "RPI-RP2")
    unreadable.chmod(0o000)
    try:
        assert not is_deployable(unreadable)
    finally:
        unreadable.chmod(0o755)


def test_the_target_is_where_uf2conv_searches(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv("USER", "sunaemon")

    assert target_mount_point("RPI-RP2") == Path("/run/media/sunaemon/RPI-RP2")


def test_an_unset_user_is_reported(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.delenv("USER", raising=False)

    with pytest.raises(Uf2VolumeError, match="USER is unset"):
        target_mount_point("RPI-RP2")


def test_waiting_gives_up_with_advice(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(mount_uf2_volume, "device_for_label", lambda _label: None)

    with pytest.raises(Uf2VolumeError, match="bootloader"):
        wait_for_device("RPI-RP2", timeout=0.0)


def test_a_mount_this_user_can_write_is_left_alone(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """The common case on a desktop session: nothing to do but report it."""
    mounted = deployable(tmp_path / "RPI-RP2")
    monkeypatch.setattr(
        mount_uf2_volume, "device_for_label", lambda _label: Path("/dev/sda1")
    )
    monkeypatch.setattr(mount_uf2_volume, "mount_point_of", lambda _device: mounted)
    monkeypatch.setattr(
        mount_uf2_volume,
        "mount",
        lambda *_args: pytest.fail("a usable mount must not be remounted"),
    )

    assert mount_volume(label="RPI-RP2", timeout=0.0, sudo="sudo") == mounted


def test_a_mount_owned_by_root_is_remounted(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """udisks over SSH mounts at /run/media/root, which qmk cannot read."""
    target = deployable(tmp_path / "sunaemon" / "RPI-RP2")
    unmounted: list[Path] = []
    mounted: list[tuple[Path, Path]] = []
    monkeypatch.setattr(
        mount_uf2_volume, "device_for_label", lambda _label: Path("/dev/sda1")
    )
    monkeypatch.setattr(
        mount_uf2_volume, "mount_point_of", lambda _device: Path("/run/media/root/X")
    )
    monkeypatch.setattr(mount_uf2_volume, "source_of_mount_point", lambda _path: None)
    monkeypatch.setattr(mount_uf2_volume, "target_mount_point", lambda _label: target)
    monkeypatch.setattr(
        mount_uf2_volume, "unmount", lambda path, _sudo: unmounted.append(path)
    )
    monkeypatch.setattr(
        mount_uf2_volume,
        "mount",
        lambda device, path, _sudo: mounted.append((device, path)),
    )

    assert mount_volume(label="RPI-RP2", timeout=0.0, sudo="sudo") == target
    assert unmounted == [Path("/run/media/root/X")]
    assert mounted == [(Path("/dev/sda1"), target)]


def test_a_mount_that_did_not_take_is_reported(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """Silence here would leave qmk waiting for a drive that never arrives."""
    monkeypatch.setattr(
        mount_uf2_volume, "device_for_label", lambda _label: Path("/dev/sda1")
    )
    monkeypatch.setattr(mount_uf2_volume, "mount_point_of", lambda _device: None)
    monkeypatch.setattr(mount_uf2_volume, "source_of_mount_point", lambda _path: None)
    monkeypatch.setattr(mount_uf2_volume, "target_mount_point", lambda _label: tmp_path)
    monkeypatch.setattr(mount_uf2_volume, "mount", lambda *_args: None)

    with pytest.raises(Uf2VolumeError, match="INFO_UF2.TXT"):
        mount_volume(label="RPI-RP2", timeout=0.0, sudo="sudo")


def test_elevation_is_skipped_when_it_would_be_pointless(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setattr(mount_uf2_volume.os, "geteuid", lambda: 1000)

    assert elevate(["umount", "/x"], "sudo") == ["sudo", "umount", "/x"]
    assert elevate(["umount", "/x"], "") == ["umount", "/x"]

    monkeypatch.setattr(mount_uf2_volume.os, "geteuid", lambda: 0)

    assert elevate(["umount", "/x"], "sudo") == ["umount", "/x"]

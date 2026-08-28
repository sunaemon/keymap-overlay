# Copyright 2026 sunaemon
# SPDX-License-Identifier: MIT
import json
import subprocess
from pathlib import Path

import pytest

from installer.release.check_hardware_gate import (
    EXPECTED_CHECK_SECTIONS,
    HardwareGateError,
    check_pull_request_event,
    validate_hardware_gate,
)

HEAD_SHA = "a" * 40
BASE_SHA = "b" * 40


def test_complete_gate_for_the_current_head_passes() -> None:
    """Complete global, platform, coverage, and lifecycle evidence passes."""
    validate_hardware_gate(complete_gate(), HEAD_SHA, frozenset())


def test_stale_candidate_is_rejected() -> None:
    """Evidence recorded for a different candidate commit is rejected."""
    with pytest.raises(HardwareGateError, match="does not match PR head"):
        validate_hardware_gate(complete_gate(candidate="c" * 40), HEAD_SHA, frozenset())


def test_missing_duplicate_unknown_and_unchecked_checks_are_rejected() -> None:
    """Malformed and incomplete checklist entries are rejected together."""
    evidence = (
        complete_gate()
        .replace(
            "- [x] **GLOBAL-01** — Result: PASS — passed\n",
            "- [ ] **GLOBAL-02** — Result: PENDING — duplicate\n"
            "- [x] **GLOBAL-99** — Result: PASS — unknown\n",
        )
        .replace(
            "- [x] **MAC-03** — Result: PASS — passed\n",
            "- [ ] **MAC-03** — Result: PENDING — pending\n",
        )
    )

    with pytest.raises(HardwareGateError) as error:
        validate_hardware_gate(evidence, HEAD_SHA, frozenset())

    message = str(error.value)
    assert "missing checks: GLOBAL-01" in message
    assert "unknown checks: GLOBAL-99" in message
    assert "duplicate checks: GLOBAL-02" in message
    assert "unchecked checks: GLOBAL-01, MAC-03" in message


def test_platform_checks_must_stay_in_their_platform_section() -> None:
    """A platform result cannot be recorded under another platform heading."""
    check = "- [x] **MAC-01** — Result: PASS — passed\n"
    evidence = (
        complete_gate()
        .replace(check, "")
        .replace(
            "### linux-x86_64-kde-wayland checks\n",
            f"### linux-x86_64-kde-wayland checks\n\n{check}",
        )
    )

    with pytest.raises(HardwareGateError, match="misplaced checks: MAC-01"):
        validate_hardware_gate(evidence, HEAD_SHA, frozenset())


def test_blank_and_incomplete_platform_rows_are_rejected() -> None:
    """Platform rows cannot omit required metadata or retain pending results."""
    evidence = complete_gate().replace(
        platform_row(
            "linux-x86_64-kde-wayland",
            "x86_64",
            "Fedora 42",
            "KDE Plasma 6 / Wayland",
            "DOIO KB16",
            "2",
            "v0.0.7 firmware",
        ),
        platform_row("linux-x86_64-kde-wayland", "x86_64", "", "Pending", "", "", ""),
    )

    with pytest.raises(HardwareGateError) as error:
        validate_hardware_gate(evidence, HEAD_SHA, frozenset())

    message = str(error.value)
    assert "linux-x86_64-kde-wayland has no OS version" in message
    assert "linux-x86_64-kde-wayland platform row has invalid KEYBOARD_ID(s)" in message


def test_platform_architecture_must_match_the_release_target() -> None:
    """A platform row cannot substitute CI or another CPU architecture."""
    evidence = complete_gate().replace(
        "| windows-arm64-wpf | arm64 |",
        "| windows-arm64-wpf | x86_64 |",
    )

    with pytest.raises(
        HardwareGateError, match="windows-arm64-wpf architecture must be arm64"
    ):
        validate_hardware_gate(evidence, HEAD_SHA, frozenset())


def test_missing_platform_and_keyboard_coverage_rows_are_rejected() -> None:
    """All required platform and concrete keyboard coverage rows are mandatory."""
    evidence = (
        complete_gate()
        .replace(
            platform_row(
                "linux-x86_64-gnome-wayland",
                "x86_64",
                "Ubuntu 26.04",
                "GNOME 49 / Wayland",
                "Insixty",
                "1",
                "v0.0.7 firmware",
            ),
            "",
        )
        .replace(
            "| simultaneous-keyboards | Insixty, DOIO KB16 | 1, 2 | windows-x86_64-wpf | PASS |\n",
            "",
        )
    )

    with pytest.raises(HardwareGateError) as error:
        validate_hardware_gate(evidence, HEAD_SHA, frozenset())

    message = str(error.value)
    assert "missing platform rows: linux-x86_64-gnome-wayland" in message
    assert "missing coverage rows: simultaneous-keyboards" in message


def test_keyboard_coverage_must_include_bundled_and_encoder_ids() -> None:
    """Coverage rows must prove every bundle and an encoder keyboard were tested."""
    evidence = (
        complete_gate()
        .replace(
            "| bundled-keyboards | Insixty, DOIO KB16 | 1, 2 | macos-arm64-appkit, linux-x86_64-kde-wayland | PASS |",
            "| bundled-keyboards | Insixty | 1 | macos-arm64-appkit, linux-x86_64-kde-wayland | PASS |",
        )
        .replace(
            "| encoder-keyboard | DOIO KB16 | 2 | linux-x86_64-kde-wayland | PASS |",
            "| encoder-keyboard | Insixty | 1 | linux-x86_64-kde-wayland | PASS |",
        )
    )

    with pytest.raises(HardwareGateError) as error:
        validate_hardware_gate(evidence, HEAD_SHA, frozenset())

    message = str(error.value)
    assert "bundled-keyboards does not cover KEYBOARD_ID(s): 2" in message
    assert "does not identify a bundled encoder keyboard" in message


def test_coverage_must_match_the_assigned_platform_rows() -> None:
    """Coverage cannot claim keyboards absent from its named platform runs."""
    evidence = complete_gate().replace(
        "| simultaneous-keyboards | Insixty, DOIO KB16 | 1, 2 | windows-x86_64-wpf | PASS |",
        "| simultaneous-keyboards | Insixty, DOIO KB16 | 1, 2 | macos-arm64-appkit | PASS |",
    )

    with pytest.raises(HardwareGateError) as error:
        validate_hardware_gate(evidence, HEAD_SHA, frozenset())

    message = str(error.value)
    assert "absent from its platform row(s): 2" in message
    assert "platform row does not list KEYBOARD_ID(s): 2" in message


def test_conditional_na_requires_an_allowed_check_and_reason() -> None:
    """Only conditional checks accept N/A, and every N/A needs a reason."""
    evidence = (
        complete_gate()
        .replace("GLOBAL-01** — Result: PASS", "GLOBAL-01** — Result: N/A:")
        .replace("MAC-02** — Result: PASS", "MAC-02** — Result: N/A: unsupported")
    )

    with pytest.raises(HardwareGateError, match="GLOBAL-01, MAC-02"):
        validate_hardware_gate(evidence, HEAD_SHA, frozenset())


def test_reasoned_global_na_results_pass() -> None:
    """Unchanged firmware may complete both release-wide checks with reasons."""
    evidence = (
        complete_gate()
        .replace(
            "GLOBAL-01** — Result: PASS",
            "GLOBAL-01** — Result: N/A: firmware and metadata unchanged",
        )
        .replace(
            "GLOBAL-02** — Result: PASS",
            "GLOBAL-02** — Result: N/A: no candidate flash required",
        )
    )

    validate_hardware_gate(evidence, HEAD_SHA, frozenset())


@pytest.mark.parametrize(
    "changed_path",
    ("firmware/layer_notify.h", "model/generate_vial.py"),
)
def test_global_na_is_rejected_when_firmware_inputs_changed(
    changed_path: str,
) -> None:
    """Firmware-affecting changes require physical flash and EEPROM passes."""
    evidence = (
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

    with pytest.raises(
        HardwareGateError, match="invalid or unreasoned results: GLOBAL-01, GLOBAL-02"
    ):
        validate_hardware_gate(evidence, HEAD_SHA, frozenset({changed_path}))


@pytest.mark.parametrize(
    "config",
    (
        '{"qmk_keyboard": "test", "encoders": "yes"}',
        '{"qmk_keyboard": "test", "encoders": 1}',
        '{"qmk_keyboard": "test", "encoders": null}',
        '{"qmk_keyboard": "test", "encoders": [{"matrix": [0]}]}',
        "{invalid json",
    ),
)
def test_invalid_bundled_keyboard_configs_are_rejected(
    tmp_path: Path, config: str
) -> None:
    """Malformed bundled encoder metadata fails the release gate closed."""
    definition = tmp_path / "1"
    definition.mkdir()
    (definition / "config.json").write_text(config, encoding="utf-8")

    with pytest.raises(HardwareGateError, match="Invalid bundled keyboard config"):
        validate_hardware_gate(
            complete_gate(),
            HEAD_SHA,
            frozenset(),
            keyboards_directory=tmp_path,
        )


def test_missing_bundled_keyboard_config_is_rejected(tmp_path: Path) -> None:
    """An unreadable bundled config is reported as a gate error."""
    (tmp_path / "1").mkdir()

    with pytest.raises(HardwareGateError, match="Invalid bundled keyboard config"):
        validate_hardware_gate(
            complete_gate(),
            HEAD_SHA,
            frozenset(),
            keyboards_directory=tmp_path,
        )


def test_lifecycle_rows_require_explicit_results_and_evidence() -> None:
    """Each platform needs upgrade, rollback, uninstall, and evidence results."""
    evidence = complete_gate().replace(
        "| windows-x86_64-wpf | PASS | PASS | PASS | local acceptance log |",
        "| windows-x86_64-wpf | PENDING | PASS | PENDING | Pending |",
    )

    with pytest.raises(HardwareGateError) as error:
        validate_hardware_gate(evidence, HEAD_SHA, frozenset())

    message = str(error.value)
    assert "windows-x86_64-wpf upgrade result must be PASS" in message
    assert "windows-x86_64-wpf uninstall result must be PASS" in message
    assert "windows-x86_64-wpf has no lifecycle evidence" in message


def test_non_release_pull_request_skips_gate(tmp_path: Path) -> None:
    """A pull request without a version change does not require release evidence."""
    event = write_event(tmp_path, body=None)
    project_file = write_project(tmp_path, "0.0.4")

    check_pull_request_event(
        event,
        project_file=project_file,
        runner=FakeRunner(previous="0.0.4"),
    )


def test_version_bump_requires_gate_for_the_current_head(tmp_path: Path) -> None:
    """A version bump passes only with complete evidence for its current head."""
    event = write_event(tmp_path, body=complete_gate())
    project_file = write_project(tmp_path, "0.0.5")

    check_pull_request_event(
        event,
        project_file=project_file,
        runner=FakeRunner(previous="0.0.4"),
    )


def test_version_bump_without_evidence_fails(tmp_path: Path) -> None:
    """A version bump with no pull request evidence fails the hardware gate."""
    event = write_event(tmp_path, body=None)
    project_file = write_project(tmp_path, "0.0.5")

    with pytest.raises(HardwareGateError, match="missing checks"):
        check_pull_request_event(
            event,
            project_file=project_file,
            runner=FakeRunner(previous="0.0.4"),
        )


def test_pull_request_gate_uses_candidate_diff_for_global_na(tmp_path: Path) -> None:
    """The pull request entrypoint requires passes for changed embedded metadata."""
    body = (
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
    event = write_event(tmp_path, body=body)
    project_file = write_project(tmp_path, "0.0.5")

    with pytest.raises(HardwareGateError, match="GLOBAL-01, GLOBAL-02"):
        check_pull_request_event(
            event,
            project_file=project_file,
            runner=FakeRunner(previous="0.0.4", changed="model/generate_vial.py\0"),
        )


class FakeRunner:
    """Return the base version requested by the gate."""

    def __init__(self, *, previous: str, changed: str = "") -> None:
        self.previous = previous
        self.changed = changed

    def __call__(self, command: list[str]) -> subprocess.CompletedProcess[str]:
        """Return project metadata or an empty changed-path set."""
        if command == ["git", "show", f"{BASE_SHA}:pyproject.toml"]:
            stdout = project(self.previous)
        else:
            assert command == [
                "git",
                "diff",
                "--no-renames",
                "--name-only",
                "-z",
                BASE_SHA,
                HEAD_SHA,
                "--",
            ]
            stdout = self.changed
        return subprocess.CompletedProcess(command, 0, stdout, "")


def complete_gate(*, candidate: str = HEAD_SHA) -> str:
    """Return complete hardware evidence for one candidate."""
    platform_check_sections = "\n".join(
        check_section(heading)
        for heading in EXPECTED_CHECK_SECTIONS
        if heading != "Platform-independent checks"
    )
    platform_rows = "".join(
        (
            platform_row(
                "macos-arm64-appkit",
                "arm64",
                "macOS 15.6",
                "AppKit / Aqua",
                "Insixty",
                "1",
                "v0.0.7 firmware",
            ),
            platform_row(
                "linux-x86_64-kde-wayland",
                "x86_64",
                "Fedora 42",
                "KDE Plasma 6 / Wayland",
                "DOIO KB16",
                "2",
                "v0.0.7 firmware",
            ),
            platform_row(
                "linux-x86_64-gnome-wayland",
                "x86_64",
                "Ubuntu 26.04",
                "GNOME 49 / Wayland",
                "Insixty",
                "1",
                "v0.0.7 firmware",
            ),
            platform_row(
                "linux-arm64-kde-wayland",
                "arm64",
                "Fedora 42",
                "KDE Plasma 6 / Wayland",
                "Insixty",
                "1",
                "v0.0.7 firmware",
            ),
            platform_row(
                "windows-x86_64-wpf",
                "x86_64",
                "Windows 11 24H2",
                "WPF / desktop",
                "Insixty, DOIO KB16",
                "1, 2",
                "v0.0.7 firmware",
            ),
            platform_row(
                "windows-arm64-wpf",
                "arm64",
                "Windows 11 24H2",
                "WPF / desktop",
                "Insixty",
                "1",
                "v0.0.7 firmware",
            ),
        )
    )
    return f"""## Hardware Release Gate

Candidate commit: `{candidate}`

### Platform test matrix

| Platform ID | Architecture | OS version | Desktop / session | Keyboard(s) | `KEYBOARD_ID(s)` | Firmware revision(s) |
| ----------- | ------------ | ---------- | ----------------- | ----------- | ---------------- | -------------------- |
{platform_rows}
{check_section("Platform-independent checks")}
### Keyboard coverage

| Coverage ID | Keyboard(s) | `KEYBOARD_ID(s)` | Platform ID(s) | Result |
| ----------- | ----------- | ---------------- | -------------- | ------ |
| bundled-keyboards | Insixty, DOIO KB16 | 1, 2 | macos-arm64-appkit, linux-x86_64-kde-wayland | PASS |
| encoder-keyboard | DOIO KB16 | 2 | linux-x86_64-kde-wayland | PASS |
| simultaneous-keyboards | Insixty, DOIO KB16 | 1, 2 | windows-x86_64-wpf | PASS |

{platform_check_sections}
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


def platform_row(
    platform_id: str,
    architecture: str,
    os_version: str,
    session: str,
    keyboard: str,
    keyboard_ids: str,
    firmware: str,
) -> str:
    """Return one platform evidence row."""
    return (
        f"| {platform_id} | {architecture} | {os_version} | {session} | {keyboard} | "
        f"{keyboard_ids} | {firmware} |\n"
    )


def check_section(heading: str) -> str:
    """Return one complete checklist section."""
    checks = "".join(
        f"- [x] **{check_id}** — Result: PASS — passed\n"
        for check_id in EXPECTED_CHECK_SECTIONS[heading]
    )
    return f"### {heading}\n\n{checks}"


def write_event(tmp_path: Path, *, body: str | None) -> Path:
    """Write a minimal GitHub pull request event."""
    event = tmp_path / "event.json"
    event.write_text(
        json.dumps(
            {
                "pull_request": {
                    "body": body,
                    "base": {"sha": BASE_SHA},
                    "head": {"sha": HEAD_SHA},
                }
            }
        )
    )
    return event


def write_project(tmp_path: Path, version: str) -> Path:
    """Write minimal current project metadata."""
    path = tmp_path / "pyproject.toml"
    path.write_text(project(version))
    return path


def project(version: str) -> str:
    """Return minimal project metadata using a version."""
    return f'[project]\nversion = "{version}"\n'

# Maintainer evidence workflow

The release owner (the person who will merge the release PR) coordinates this
procedure and reviews the combined evidence. Platform testers run the
[hardware test procedure](hardware-release-testing.md) on their own machines.
Use this workflow for every release after CI passes and before merging.

## 1. Freeze and distribute the candidate

Record the release PR's full head SHA and assign testers to macOS ARM64,
Linux x86_64 KDE Wayland, Linux x86_64 GNOME Wayland, and Windows x86_64.
Share the SHA, release PR URL, previous release version, firmware changes,
keyboard assignments, and a location for uploading evidence bundles. Assign
one tester to each release-wide firmware and keyboard-coverage requirement.
KDE and GNOME share Linux device checks and lifecycle results; they need
separate renderer and login observations.

Each tester checks out that SHA, verifies a clean worktree, and follows the
preparation, installation, physical checks, and lifecycle steps in the hardware
procedure. Keep evidence outside the checkout so it does not make the candidate
dirty or disappear during `make clean`. Record the SHA before testing; do not
substitute a newer checkout's SHA when recording an older run.

## 2. Capture each run and reuse its metadata

### Guided collection

From the clean candidate checkout, the guided command runs available session
and physical-report helpers, then asks for the remaining local observations.
It creates the portable bundles and summary automatically. Use a new directory
outside the checkout for each attempt:

```bash
uv run python -m installer.release.run_hardware_checks \
  --output "$HOME/kmo-evidence/current-attempt" \
  --tester 'Your name' \
  --keyboard 'Insixty|1|actual firmware revision' \
  --keyboard 'DOIO KB16|2|actual firmware revision'
```

The same entry point runs in native Windows PowerShell:

```powershell
uv run python -m installer.release.run_hardware_checks `
  --output "$env:USERPROFILE/kmo-evidence/current-attempt" `
  --tester 'Your name' `
  --keyboard 'Insixty|1|actual firmware revision'
```

Add `--plan` to preview the helper commands without running them or creating
evidence. The runner detects the candidate SHA, OS, architecture, and Linux
desktop/session. Keyboard names and firmware revisions are declarations: the
HIL interface does not reliably expose a firmware commit. Confirm the USB/ID
identity during the physical check. Match the existing helpers' `KMO_HIL_*`
settings to the participating boards; defaults expect both bundled keyboards
and physical reports `1:1 1:2 2:3`. The runner preserves those settings.

On macOS it runs physical reports for `GLOBAL-03` followed by the session HIL
helper. On KDE it runs the virtual-device session helper. On GNOME and Windows
it uses the guided observation path because no session HIL helper supports
those renderers.
No new renderer coverage is implied by wrapping these helpers.

Complete the hardware procedure's prerequisites first: candidate HIL firmware
and macOS permissions, Linux device access, and KDE `/dev/uhid` access. The
runner confirms before starting helpers that install/restart services or
temporarily edit and restore Vial bindings. Observe physical typing before
starting and again afterward. It imports only complete checks after the
helper exits successfully, including cleanup. The physical-report helper
imports `GLOBAL-03` independently of platform `*-08` results.

For each remaining local check, enter `PASS`, `FAIL`, or `SKIP` (the default).
A recorded outcome requires a description of what you did and observed.
Checklist wording comes from the release template. Skip anything not actually
performed, especially login checks if you have not signed out and back in.
Use the standalone recorder below for later observations. Firmware flashing,
login, lifecycle operations, and release-wide keyboard coverage remain the
explicit steps documented in the hardware procedure.

Failures stop the run and preserve its transcripts and completed records.
The summary marks interrupted runs as stopped. Changing the candidate or
dirtying its checkout stops further recording. A completed prompting session
does not mean the release gate passes: inspect missing items in `summary.md`.
Transfer the entire run directory, including `helper-logs`, through steps 4–6.

### Standalone recording

Use the commands below for independently collected results, lifecycle logs,
later login observations, or checks skipped during guided collection.

Keep the command, exit status, tester name, date, platform/session, keyboard
identity and firmware revision, and observations in the transcript. Existing
HIL targets print their transcript path and candidate. Wait for the command to
finish, including cleanup: a success message printed before a failed cleanup
is not a successful run. Record failures explicitly. For manual checks, write
the observations and commands into a saved text transcript. A physical result
must describe the action actually performed and observed.

On macOS or Linux, define the shared arguments once in Bash. Replace the sample
SHA, OS version, keyboard identities, and firmware revision with tested values:

```bash
KMO_CANDIDATE_SHA="$(git rev-parse HEAD)"
KMO_EVIDENCE_DIR="$HOME/kmo-evidence/$KMO_CANDIDATE_SHA/macos-arm64-appkit"
kmo_metadata=(
  --candidate-sha "$KMO_CANDIDATE_SHA"
  --platform-id macos-arm64-appkit
  --os-version "$(sw_vers -productVersion)"
  --session 'AppKit / Aqua'
  --keyboard 'Insixty|1|tested firmware revision'
  --keyboard 'DOIO KB16|2|tested firmware revision'
)
```

On Linux, use this equivalent setup (substitute GNOME's platform and desktop
version when gathering GNOME observations):

```bash
KMO_CANDIDATE_SHA="$(git rev-parse HEAD)"
KMO_PLATFORM=linux-x86_64-kde-wayland
KMO_EVIDENCE_DIR="$HOME/kmo-evidence/$KMO_CANDIDATE_SHA/$KMO_PLATFORM"
kmo_metadata=(
  --candidate-sha "$KMO_CANDIDATE_SHA"
  --platform-id "$KMO_PLATFORM"
  --os-version 'actual distribution and version from /etc/os-release'
  --session 'actual KDE Plasma version / Wayland'
  --keyboard 'Insixty|1|tested firmware revision'
)
```

Update the arguments when devices, firmware, or session change. The keyboard
list describes participating physical devices, not the virtual fixture used by
the Linux integration test.

| Platform                     | Helper / import profile                                  | Results imported               | Remaining tester work                                                                                                               |
| ---------------------------- | -------------------------------------------------------- | ------------------------------ | ----------------------------------------------------------------------------------------------------------------------------------- |
| `macos-arm64-appkit`         | physical reports + `make test-hardware-session-macos`    | `GLOBAL-03`, `MAC-01`–`MAC-04` | Visuals, displays, typing/USB identity, repeated transitions, disconnects, first physical press after login, coverage and lifecycle |
| `linux-x86_64-kde-wayland`   | `make test-hardware-session-linux` / `linux-kde-session` | `LX-02`, `LX-03`, `LX-08`      | Device checks, startup/log inspection, pointer/window order, visuals/displays and login                                             |
| `linux-x86_64-gnome-wayland` | No complete-check import profile                         | None                           | GNOME startup, window safety, visuals/displays and login; cite shared Linux device results separately                               |
| `windows-x86_64-win32`       | No complete-check import profile                         | None                           | All Win32 checks, including typing through the second and later shows, and lifecycle                                                |

The physical-report helper supplies `GLOBAL-03`; deterministic session or
manual backend evidence supplies each platform's `*-08`. The macOS login helper
also needs an explicit observation of the first physical layer press after
sign-in. Neither helper automatically completes these checks. See the
[coverage map](release-test-coverage.md) for the full boundaries.

After a successful macOS session run, substitute its actual transcript path:

```bash
uv run python -m installer.release.record_hardware_evidence \
  "${kmo_metadata[@]}" \
  --transcript /path/to/macos-session.log \
  --profile macos-session \
  --output "$KMO_EVIDENCE_DIR/session.json"
```

Use the same command on KDE with its metadata and `--profile linux-kde-session`.
The importer requires the transcript's candidate line to match the recorded
SHA and recognizes only the documented success marker. The tester still
checks the command's final exit status and the complete transcript.

## 3. Add explicit observations and lifecycle results

Repeat `--check` for every completed check in a manual transcript. Use `manual`
for visual judgment and `physical` for observed physical actions. Quote each
argument containing `|`. For example, after performing and recording `MAC-05`:

```bash
uv run python -m installer.release.record_hardware_evidence \
  "${kmo_metadata[@]}" \
  --transcript /path/to/visual-observations.txt \
  --check 'MAC-05|PASS|manual|Compared every layer with live Vial; no clipping or incorrect labels' \
  --output "$KMO_EVIDENCE_DIR/visual.json"
```

For compound checks, retain both supporting transcripts and identify them in
the tester's observation transcript. For a failed check use `FAIL` with the
observed error. Never copy the example PASS without performing the check.

On Windows, use PowerShell's argument array to reuse the same metadata. Save
command output with `Start-Transcript` / `Stop-Transcript`, include native
command exit codes (`$LASTEXITCODE`), and record the physical observations:

```powershell
$KmoCandidateSha = (git rev-parse HEAD).Trim()
$KmoEvidenceDir = Join-Path $env:USERPROFILE "kmo-evidence/$KmoCandidateSha/windows-x86_64-win32"
$KmoMetadata = @(
  '--candidate-sha', $KmoCandidateSha,
  '--platform-id', 'windows-x86_64-win32',
  '--os-version', [System.Environment]::OSVersion.VersionString,
  '--session', 'Win32 / Windows 11 desktop',
  '--keyboard', 'Insixty|1|tested firmware revision'
)
uv run python -m installer.release.record_hardware_evidence @KmoMetadata `
  --transcript C:/evidence/window-observations.txt `
  --check 'WIN-04|PASS|physical|Typing and clicks reached the editor on first, second, and later shows' `
  --output "$KmoEvidenceDir/window.json"
```

After all three lifecycle operations in the hardware procedure have run,
record their individual outcomes using the same metadata and transcript:

```bash
uv run python -m installer.release.record_hardware_evidence \
  "${kmo_metadata[@]}" \
  --transcript /path/to/lifecycle.log \
  --lifecycle 'macos-arm64-appkit|PASS|PASS|PASS' \
  --output "$KMO_EVIDENCE_DIR/lifecycle.json"
```

The fields are platform ID, upgrade, rollback, uninstall. Use `linux-x86_64`
for the shared Linux row and `windows-x86_64-win32` on Windows (with
`@KmoMetadata` and PowerShell continuation syntax). Each operation accepts
`PASS` or `FAIL`. Do not enter PASS for an operation not yet run; retain the
partial transcript and finish the row later. Login stays in each platform's
`*-10` check. CI rollback results alone do not fill a local lifecycle row.

## 4. Transfer portable evidence bundles

The record command creates `name.json` and a copy of its source transcript,
`name.log`, together. The JSON uses the relative transcript filename so the
pair works after moving from Windows, Linux, or macOS. Existing output files
are not overwritten; use distinct names for retries. Keep original and
supporting logs, particularly for compound checks and failed runs.

Upload the entire per-platform directory to the agreed shared location, or
archive it and attach it to the release PR. Include a tester note naming the
candidate, platform, operations completed, failures, and anything still
missing. Do not send just the JSON. The release owner extracts all directories
under one candidate directory on the review machine, preserving each pair.
Use durable attachment URLs in the PR so another reviewer can retrieve the
transcripts; a path on the tester's machine is not a handoff.

## 5. Aggregate and reconcile on the release owner's machine

Run the collector from the candidate checkout. Explicitly include every
record being reviewed, repeating `--record` as needed:

```bash
uv run python -m installer.release.collect_hardware_evidence \
  --candidate-sha FULL_RELEASE_PR_HEAD_SHA \
  --record /path/to/evidence/macos-arm64-appkit/session.json \
  --record /path/to/evidence/linux-x86_64-kde-wayland/session.json \
  --record /path/to/evidence/linux-x86_64-gnome-wayland/observations.json \
  --record /path/to/evidence/windows-x86_64-win32/window.json \
  --output /path/to/evidence/hardware-summary.md
```

Start with `Problems`, then inspect every source transcript and its run
metadata. `STALE` requires a run for the current SHA; never relabel an old
record. `INCOMPLETE` requires the missing transcript or tester observation.
`MISSING` requires the corresponding check or lifecycle run. `FAIL` requires
investigation and a repeat run. A failing result takes precedence over a pass
when both are included. Keep the failure and the explanation of its resolution
in the evidence archive; after reviewing a successful replacement, explicitly
select the replacement record for the final summary and document that choice
in the PR. The collector does not silently select the newest attempt.

The summary is not a gate verdict. Its exit status reports whether collection
succeeded, not whether the release is ready. Reconcile the four platform rows
and all three keyboard coverage rows separately in the template. Record every
bundled keyboard, physical encoder direction/push observations, and the
simultaneous-device ownership run. These cannot be inferred from individual
check PASS results. Record `GLOBAL-01`/`GLOBAL-02` explicitly; where permitted,
put the reasoned `N/A` directly in the PR. Record `GLOBAL-03` from a reviewed
physical-report transcript. A conditional exception remains MISSING in
the collector and must be reconciled in review; the gate determines whether
the release delta allows it.

## 6. Update and review the release PR

The release owner transfers the reviewed metadata, check results, coverage,
and lifecycle results into the template without changing stable IDs or table
headers. Add the summary and links to the uploaded evidence bundles, including
tester names and any replacement-run decisions. Confirm the PR head still
equals the tested SHA, all required renderers are represented, and every
physical result has an explicit tester observation.

Editing the PR reruns `hardware-release-gate`. Resolve its failures and wait
for it and the other required checks to pass before merge. If the candidate
changes, wait for CI, distribute the new SHA, and collect evidence for that
candidate; the old records remain visibly stale. Missing hardware or a required
renderer is a release-owner blocker, not a reason to promote simulation output.

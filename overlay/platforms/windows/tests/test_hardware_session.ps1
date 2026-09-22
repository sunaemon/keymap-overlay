# Copyright 2026 sunaemon
# SPDX-License-Identifier: MIT

$ErrorActionPreference = 'Stop'

$script:projectDirectory = (Resolve-Path (Join-Path $PSScriptRoot '..\..\..\..')).Path
$script:driver = Join-Path $projectDirectory 'target\release\keymap-overlay-hil.exe'
$script:overlay = Join-Path $env:LOCALAPPDATA 'Programs\keymap-overlay\keymap-overlay.exe'
$script:overlayLog = Join-Path $env:LOCALAPPDATA 'keymap-overlay\logs\overlay.log'
$script:uiProbeSource = Join-Path $PSScriptRoot 'HilUiProbe.cs'
$script:keyboardId = if ($env:KMO_HIL_KEYBOARD_ID) { $env:KMO_HIL_KEYBOARD_ID } else { '1' }
$script:secondaryKeyboardId = if ($env:KMO_HIL_SECONDARY_KEYBOARD_ID) { $env:KMO_HIL_SECONDARY_KEYBOARD_ID } else { '2' }
$script:encoderKeyboardId = if ($env:KMO_HIL_ENCODER_KEYBOARD_ID) { $env:KMO_HIL_ENCODER_KEYBOARD_ID } else { '2' }
$script:encoderIndex = if ($env:KMO_HIL_ENCODER_INDEX) { $env:KMO_HIL_ENCODER_INDEX } else { '0' }
$script:primaryLayer = if ($env:KMO_HIL_PRIMARY_LAYER) { $env:KMO_HIL_PRIMARY_LAYER } else { '1' }
$script:secondaryLayer = if ($env:KMO_HIL_SECONDARY_LAYER) { $env:KMO_HIL_SECONDARY_LAYER } else { '2' }
$script:transcriptDirectory = if ($env:KMO_HIL_LOG_DIR) {
    $env:KMO_HIL_LOG_DIR
}
else {
    Join-Path $env:LOCALAPPDATA 'keymap-overlay\hil'
}
$script:stateFile = Join-Path $transcriptDirectory 'windows-state.log'
$script:transcript = Join-Path $transcriptDirectory (
    'windows-session-' + (Get-Date -Format 'yyyyMMdd-HHmmss') + '.log'
)
$script:overlayProcess = $null
$script:restoreRequired = $false
$script:testRow = ''
$script:testColumn = ''
$script:originalKeycode = ''
$script:encoderOriginalKeycode = ''
$script:encoderRestoreRequired = $false

function Invoke-WindowsHardwareSession {
    Confirm-Prerequisites
    New-Item -ItemType Directory -Path $transcriptDirectory -Force | Out-Null
    Remove-Item -LiteralPath $stateFile -Force -ErrorAction SilentlyContinue
    Start-Transcript -LiteralPath $transcript -Force | Out-Null
    try {
        $candidate = Invoke-GitForOutput @('rev-parse', 'HEAD')
        Write-Output "Candidate: $candidate"
        Invoke-HilDriver @('devices') | Write-Output
        Invoke-HilDriver @('probe', '--keyboard-id', $keyboardId) | Write-Output
        Invoke-HilDriver @('probe', '--keyboard-id', $secondaryKeyboardId) | Write-Output

        $coordinates = Invoke-HilDriver @(
            'find-transparent', '--keyboard-id', $keyboardId, '--layer', $primaryLayer
        )
        if ($coordinates -notmatch '^row=(\d+) column=(\d+) original=(0x[0-9A-F]{4})$') {
            throw "Could not parse transparent key coordinates: $coordinates"
        }
        $script:testRow = $Matches[1]
        $script:testColumn = $Matches[2]
        $script:originalKeycode = $Matches[3]

        Stop-Overlay
        $env:KEYMAP_OVERLAY_E2E_STATE_FILE = $stateFile
        Start-TestOverlay
        Test-LayerTransitions
        Test-RestartRead
        Test-WindowInput
        Write-Output (
            'PASS: Windows live Vial restart read, ten Raw HID cycles, nested ordering, ' +
            'restoration, Win32 state, focus, standard key input, click-through, and topmost'
        )
        Write-Output "Transcript: $transcript"
    }
    finally {
        try {
            Restore-Session
        }
        finally {
            Stop-Transcript | Out-Null
        }
    }
}

function Confirm-Prerequisites {
    if (-not [Environment]::Is64BitOperatingSystem -or $env:PROCESSOR_ARCHITECTURE -ne 'AMD64') {
        throw 'The Windows hardware release row requires Windows x86_64'
    }
    if (Invoke-GitForOutput @('status', '--short')) {
        throw 'Candidate worktree is not clean'
    }
    foreach ($path in @($driver, $overlay, $uiProbeSource)) {
        if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
            throw "Required executable is missing: $path"
        }
    }
}

function Test-LayerTransitions {
    Invoke-HilDriver @(
        'layer', '--keyboard-id', $keyboardId, '--layer', $primaryLayer, '--state', 'press'
    ) | Out-Null
    Wait-ForState 'lower layer show' "show keyboard=$keyboardId layers=[$primaryLayer]" 1
    Invoke-HilDriver @(
        'layer', '--keyboard-id', $keyboardId, '--layer', $secondaryLayer, '--state', 'press'
    ) | Out-Null
    Wait-ForState 'numeric precedence' `
        "show keyboard=$keyboardId layers=[$primaryLayer, $secondaryLayer]" 1
    Invoke-HilDriver @(
        'layer', '--keyboard-id', $keyboardId, '--layer', $secondaryLayer, '--state', 'release'
    ) | Out-Null
    Wait-ForState 'lower layer restoration' "show keyboard=$keyboardId layers=[$primaryLayer]" 2
    Invoke-HilDriver @(
        'layer', '--keyboard-id', $keyboardId, '--layer', $primaryLayer, '--state', 'release'
    ) | Out-Null
    Wait-ForState 'final hide' 'hide size=1x1' 1

    for ($cycle = 1; $cycle -le 10; $cycle++) {
        Invoke-HilDriver @(
            'layer', '--keyboard-id', $keyboardId, '--layer', $primaryLayer, '--state', 'press'
        ) | Out-Null
        Wait-ForState "cycle $cycle show" `
            "show keyboard=$keyboardId layers=[$primaryLayer]" ($cycle + 2)
        Invoke-HilDriver @(
            'layer', '--keyboard-id', $keyboardId, '--layer', $primaryLayer, '--state', 'release'
        ) | Out-Null
        Wait-ForState "cycle $cycle hide" 'hide size=1x1' ($cycle + 1)
    }
}

function Test-RestartRead {
    Stop-Overlay
    Invoke-HilDriver @(
        'set-keycode', '--keyboard-id', $keyboardId, '--layer', '0',
        '--row', $testRow, '--column', $testColumn, '--keycode', '0x0068'
    ) | Out-Null
    $script:restoreRequired = $true
    Confirm-Keycode '0x0068'
    Start-TestOverlay
    Invoke-HilDriver @(
        'layer', '--keyboard-id', $keyboardId, '--layer', $primaryLayer, '--state', 'press'
    ) | Out-Null
    Wait-ForState 'the restarted overlay to display the F13 edit' 'first_label=Some(["F13"])' 1
    Invoke-HilDriver @(
        'layer', '--keyboard-id', $keyboardId, '--layer', $primaryLayer, '--state', 'release'
    ) | Out-Null
    Wait-ForState 'edited model hide' 'hide size=1x1' 12

    Stop-Overlay
    Restore-Keycode
    Start-TestOverlay
    Invoke-HilDriver @(
        'layer', '--keyboard-id', $keyboardId, '--layer', $primaryLayer, '--state', 'press'
    ) | Out-Null
    Wait-ForState 'restored model show' "show keyboard=$keyboardId layers=[$primaryLayer]" 14
    Invoke-HilDriver @(
        'layer', '--keyboard-id', $keyboardId, '--layer', $primaryLayer, '--state', 'release'
    ) | Out-Null
    Wait-ForState 'restored model hide' 'hide size=1x1' 13
}

function Test-WindowInput {
    Stop-Overlay
    $script:encoderOriginalKeycode = Invoke-HilDriver @(
        'get-encoder', '--keyboard-id', $encoderKeyboardId, '--layer', '0',
        '--index', $encoderIndex, '--direction', 'ccw'
    )
    Invoke-HilDriver @(
        'set-encoder', '--keyboard-id', $encoderKeyboardId, '--layer', '0',
        '--index', $encoderIndex, '--direction', 'ccw', '--keycode', '0x0004'
    ) | Out-Null
    $script:encoderRestoreRequired = $true
    Confirm-EncoderKeycode '0x0004'
    Start-TestOverlay

    Add-Type -AssemblyName System.Windows.Forms
    Add-Type -AssemblyName System.Drawing
    Add-Type -Path $uiProbeSource -ReferencedAssemblies @(
        'System.Windows.Forms.dll', 'System.Drawing.dll'
    )
    [KeymapOverlay.Hil.WindowsUiProbe]::Run(
        $driver,
        $overlayProcess.Id,
        [int]$keyboardId,
        [int]$primaryLayer,
        [int]$encoderKeyboardId,
        [int]$encoderIndex
    ) | Write-Output

    Stop-Overlay
    Restore-EncoderKeycode
    Start-TestOverlay
}

function Start-TestOverlay {
    $script:overlayProcess = Start-Process -FilePath $overlay `
        -ArgumentList @('--log-out', $overlayLog) -WindowStyle Hidden -PassThru
    Start-Sleep -Seconds 2
    if ($overlayProcess.HasExited) {
        throw 'Exact-head overlay exited during startup'
    }
}

function Stop-Overlay {
    Get-Process keymap-overlay -ErrorAction SilentlyContinue | Stop-Process -Force
    Start-Sleep -Milliseconds 250
    $script:overlayProcess = $null
}

function Wait-ForState(
    [string]$Description,
    [string]$Pattern,
    [int]$Count
) {
    for ($attempt = 0; $attempt -lt 300; $attempt++) {
        if ($null -ne $overlayProcess -and $overlayProcess.HasExited) {
            throw "Overlay exited while waiting for $Description"
        }
        $matches = if (Test-Path -LiteralPath $stateFile) {
            @(Select-String -LiteralPath $stateFile -SimpleMatch $Pattern).Count
        }
        else {
            0
        }
        if ($matches -ge $Count) {
            return
        }
        Start-Sleep -Milliseconds 50
    }
    $state = if (Test-Path -LiteralPath $stateFile) {
        Get-Content -Raw -LiteralPath $stateFile
    }
    else {
        '<missing>'
    }
    throw "Timed out waiting for $Description. State:`n$state"
}

function Confirm-Keycode([string]$Expected) {
    $actual = Invoke-HilDriver @(
        'get-keycode', '--keyboard-id', $keyboardId, '--layer', '0',
        '--row', $testRow, '--column', $testColumn
    )
    if ($actual -ne $Expected) {
        throw "Expected Vial keycode $Expected, found $actual"
    }
}

function Restore-Keycode {
    if (-not $restoreRequired) {
        return
    }
    Invoke-HilDriver @(
        'set-keycode', '--keyboard-id', $keyboardId, '--layer', '0',
        '--row', $testRow, '--column', $testColumn, '--keycode', $originalKeycode
    ) | Out-Null
    Confirm-Keycode $originalKeycode
    $script:restoreRequired = $false
}

function Confirm-EncoderKeycode([string]$Expected) {
    $actual = Invoke-HilDriver @(
        'get-encoder', '--keyboard-id', $encoderKeyboardId, '--layer', '0',
        '--index', $encoderIndex, '--direction', 'ccw'
    )
    if ($actual -ne $Expected) {
        throw "Expected Vial encoder keycode $Expected, found $actual"
    }
}

function Restore-EncoderKeycode {
    if (-not $encoderRestoreRequired) {
        return
    }
    Invoke-HilDriver @(
        'set-encoder', '--keyboard-id', $encoderKeyboardId, '--layer', '0',
        '--index', $encoderIndex, '--direction', 'ccw',
        '--keycode', $encoderOriginalKeycode
    ) | Out-Null
    Confirm-EncoderKeycode $encoderOriginalKeycode
    $script:encoderRestoreRequired = $false
}

function Restore-Session {
    Remove-Item Env:KEYMAP_OVERLAY_E2E_STATE_FILE -ErrorAction SilentlyContinue
    $cleanupError = $null
    try {
        $cleanupSteps = @(
            {
                Invoke-HilDriver @(
                    'layer', '--keyboard-id', $keyboardId, '--layer', $primaryLayer,
                    '--state', 'release'
                ) | Out-Null
            },
            {
                Invoke-HilDriver @(
                    'layer', '--keyboard-id', $keyboardId, '--layer', $secondaryLayer,
                    '--state', 'release'
                ) | Out-Null
            },
            { Stop-Overlay },
            { Restore-Keycode },
            { Restore-EncoderKeycode }
        )
        foreach ($cleanupStep in $cleanupSteps) {
            try {
                & $cleanupStep
            }
            catch {
                if ($null -eq $cleanupError) {
                    $cleanupError = $_
                }
            }
        }
    }
    finally {
        try {
            Start-Process -FilePath $overlay `
                -ArgumentList @('--log-out', $overlayLog) -WindowStyle Hidden
        }
        catch {
            if ($null -eq $cleanupError) {
                $cleanupError = $_
            }
        }
    }
    if ($null -ne $cleanupError) {
        throw $cleanupError
    }
}

function Invoke-HilDriver([string[]]$Arguments) {
    $output = & $driver @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw "HIL driver failed: $($Arguments -join ' ')"
    }
    return $output
}

function Invoke-GitForOutput([string[]]$Arguments) {
    $output = & git -C $projectDirectory @Arguments | Out-String
    if ($LASTEXITCODE -ne 0) {
        throw "git failed: $($Arguments -join ' ')"
    }
    return $output.Trim()
}

Invoke-WindowsHardwareSession

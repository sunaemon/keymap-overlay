$ErrorActionPreference = "Stop"

$projectDirectory = (Resolve-Path (Join-Path $PSScriptRoot "..\..\..\..")).Path
$overlay = if ($env:KEYMAP_OVERLAY_E2E_OVERLAY) {
    $env:KEYMAP_OVERLAY_E2E_OVERLAY
} else {
    Join-Path $projectDirectory "target\release\keymap-overlay.exe"
}
$testDirectory = Join-Path ([IO.Path]::GetTempPath()) ("keymap-overlay-e2e-" + [guid]::NewGuid())
$stateFile = Join-Path $testDirectory "state"
$outputFile = Join-Path $testDirectory "overlay.out.log"
$errorFile = Join-Path $testDirectory "overlay.err.log"
$process = $null
$stateWaitAttempts = 300

Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;

public static class KeymapOverlayWindow {
    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    public static extern IntPtr FindWindow(string className, string windowName);

    [DllImport("user32.dll")]
    public static extern bool PostMessage(IntPtr window, uint message, IntPtr parameter, IntPtr data);
}
'@

function Fail-Test([string]$message) {
    Write-Error "Windows E2E failure: $message"
}

function Close-Overlay {
    if ($process.HasExited) {
        return
    }
    $window = [KeymapOverlayWindow]::FindWindow("KeymapOverlayWindow", "Keymap Overlay")
    if ($window -ne [IntPtr]::Zero) {
        [void][KeymapOverlayWindow]::PostMessage(
            $window,
            0x0010,
            [IntPtr]::Zero,
            [IntPtr]::Zero
        )
        if ($process.WaitForExit(5000)) {
            return
        }
    }
    if (-not $process.HasExited) {
        Stop-Process -Id $process.Id -Force
        $process.WaitForExit()
    }
}

function Wait-ForState([string]$description, [string]$pattern, [int]$count = 1) {
    for ($attempt = 0; $attempt -lt $stateWaitAttempts; $attempt++) {
        if ($process.HasExited) {
            Fail-Test "overlay exited while waiting for $description"
        }
        $matches = if (Test-Path $stateFile) {
            @(Select-String -Path $stateFile -SimpleMatch $pattern).Count
        } else {
            0
        }
        if ($matches -ge $count) {
            return
        }
        Start-Sleep -Milliseconds 50
    }
    Fail-Test "timed out waiting for $description"
}

New-Item -ItemType Directory -Path $testDirectory | Out-Null
try {
    $legacyGenerator = Join-Path (Split-Path -Parent $overlay) "keymap-overlay-generator.exe"
    if (Test-Path -LiteralPath $legacyGenerator -PathType Leaf) {
        Fail-Test "legacy model generator must not be installed beside the Windows executable"
    }
    $env:KEYMAP_OVERLAY_E2E_STATE_FILE = $stateFile
    $env:KEYMAP_OVERLAY_PREFERENCES_FILE = Join-Path $testDirectory "preferences.json"
    $env:KEYMAP_OVERLAY_E2E_EXERCISE_TRAY = "1"

    $process = Start-Process -FilePath $overlay `
        -ArgumentList "--simulate", "1:2" `
        -RedirectStandardOutput $outputFile -RedirectStandardError $errorFile -PassThru

    Wait-ForState "the composed layer to be attached" `
        "show keyboard=1 layers=[2] size=202x152 keys=2 encoders=0 held=1"
    Wait-ForState "the simulated release to detach and hide the layer" "hide size=1x1"
    Wait-ForState "the next simulated press to attach the layer again" `
        "show keyboard=1 layers=[2] size=202x152 keys=2 encoders=0 held=1" 2

    if ($process.HasExited) {
        Fail-Test "overlay exited while processing Windows state transitions"
    }
    Close-Overlay
    $process = $null
    Write-Output "Windows native E2E test passed"
} catch {
    if (Test-Path $outputFile) {
        Write-Warning "Overlay output:`n$((Get-Content -Raw $outputFile))"
    }
    if (Test-Path $errorFile) {
        Write-Warning "Overlay errors:`n$((Get-Content -Raw $errorFile))"
    }
    if (Test-Path $stateFile) {
        Write-Error "Observed Windows states:`n$((Get-Content -Raw $stateFile))"
    }
    throw
} finally {
    Remove-Item Env:KEYMAP_OVERLAY_E2E_STATE_FILE -ErrorAction SilentlyContinue
    Remove-Item Env:KEYMAP_OVERLAY_PREFERENCES_FILE -ErrorAction SilentlyContinue
    Remove-Item Env:KEYMAP_OVERLAY_E2E_EXERCISE_TRAY -ErrorAction SilentlyContinue
    if ($null -ne $process -and -not $process.HasExited) {
        Close-Overlay
    }
    Remove-Item -LiteralPath $testDirectory -Recurse -Force -ErrorAction SilentlyContinue
}
